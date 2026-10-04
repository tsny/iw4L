use ::inventory::ItemInstance;
use math_iw4::angle_vectors;

use crate::bullet_collision::{AuthorityDObjState, MASK_PLAYER_SOLID};
use crate::frame::FrameWorld;
use crate::world::SimState;
use crate::{AuthorityModelOwner, ClientId, EntityCollisionCapabilities, ScriptModelId};

pub const MAX_WORLD_ITEMS: usize = 32;

/// Presence IDs for dropped stacks. GSC-spawned presences stay below this.
pub(crate) const WORLD_ITEM_PRESENCE_BASE: u32 = 0x7000_0000;

const DROP_FORWARD: f32 = 24.0;
const DROP_PROBE_UP: f32 = 16.0;
const DROP_PROBE_DOWN: f32 = 96.0;

/// A single item stack lying in the world, drawn with its authored model.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldItem {
    pub presence: ScriptModelId,
    pub item: ItemInstance,
    pub origin: [f32; 3],
    pub yaw: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldItems {
    pub next_serial: u32,
    pub items: Vec<WorldItem>,
}

impl Default for WorldItems {
    fn default() -> Self {
        Self {
            next_serial: 1,
            items: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldItemDropRejectReason {
    NoWorldModel,
    CapacityExhausted,
    NoEntity,
}

impl WorldItemDropRejectReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoWorldModel => "item has no world model on this map",
            Self::CapacityExhausted => "too many items are on the ground",
            Self::NoEntity => "no free world entity",
        }
    }
}

pub(crate) fn world_model_name(item: &ItemInstance) -> Option<&'static str> {
    crate::loot_catalog()
        .definition(item.definition)?
        .world_model
        .as_ref()
        .map(|key| key.name.as_str())
}

/// Checks that `item` can be dropped by `client` without changing state.
pub(crate) fn check_drop(
    world: &FrameWorld,
    item: &ItemInstance,
) -> Result<(), WorldItemDropRejectReason> {
    let model = world_model_name(item).ok_or(WorldItemDropRejectReason::NoWorldModel)?;
    if world.model_capability(model).flatten().is_none() {
        return Err(WorldItemDropRejectReason::NoWorldModel);
    }
    if world.world_items().items.len() >= MAX_WORLD_ITEMS {
        return Err(WorldItemDropRejectReason::CapacityExhausted);
    }
    Ok(())
}

/// Places `item` on the floor in front of `client`. Call `check_drop` first.
pub(crate) fn drop_in_front(
    world: &mut FrameWorld,
    client: ClientId,
    item: ItemInstance,
) -> Result<ScriptModelId, WorldItemDropRejectReason> {
    let ps = world
        .player(client)
        .copied()
        .expect("dropping client exists");
    let (forward, _, _) = angle_vectors([0.0, ps.viewangles[1], 0.0]);
    let waist = [ps.origin[0], ps.origin[1], ps.origin[2] + DROP_PROBE_UP];
    let ahead = [
        waist[0] + forward[0] * DROP_FORWARD,
        waist[1] + forward[1] * DROP_FORWARD,
        waist[2],
    ];

    // Drop at the feet when a wall is in the way, then settle onto the floor.

    let reach = world.trace_world(waist, ahead, [0.0; 3], [0.0; 3], MASK_PLAYER_SOLID);
    let start = if reach.fraction < 1.0 { waist } else { ahead };
    let floor = [start[0], start[1], start[2] - DROP_PROBE_DOWN];
    let settle = world.trace_world(start, floor, [0.0; 3], [0.0; 3], MASK_PLAYER_SOLID);
    let origin = if settle.fraction < 1.0 && settle.startsolid == 0 {
        settle.endpos
    } else {
        ps.origin
    };
    spawn(world, item, origin, ps.viewangles[1])
}

fn spawn(
    world: &mut FrameWorld,
    item: ItemInstance,
    origin: [f32; 3],
    yaw: f32,
) -> Result<ScriptModelId, WorldItemDropRejectReason> {
    let serial = world.world_items().next_serial;
    let presence = ScriptModelId::from_wire(
        WORLD_ITEM_PRESENCE_BASE
            .checked_add(serial)
            .ok_or(WorldItemDropRejectReason::CapacityExhausted)?,
    );
    let row = WorldItem {
        presence,
        item,
        origin,
        yaw,
    };
    let number = world
        .spawn_script_mover(presence, origin, [0.0, yaw, 0.0])
        .map_err(|_| WorldItemDropRejectReason::NoEntity)?;
    if let Some(mover) = world.script_mover_mut_by_number(number) {
        mover.nonsolid = true;
    }
    insert_owner(world, &row);
    let items = world.world_items_mut();
    items.next_serial = serial + 1;
    items.items.push(row);
    Ok(presence)
}

fn insert_owner(world: &mut SimState, row: &WorldItem) {
    let Some(model) = world_model_name(&row.item) else {
        return;
    };
    let capability = world.model_capability(model).flatten();
    let angles = [0.0, row.yaw, 0.0];
    let mut owner = EntityCollisionCapabilities::current_tick(
        AuthorityModelOwner::ScriptModel(row.presence),
        Some(AuthorityDObjState::at_pose(
            model, capability, row.origin, angles,
        )),
        Vec::new(),
    );
    owner.solid = false;
    owner.followed_pose = Some((row.origin, angles));
    world.insert_collision_owner(owner);
}

/// Removes the stack and its world entity, returning the item it held.
pub(crate) fn remove(world: &mut FrameWorld, presence: ScriptModelId) -> Option<ItemInstance> {
    let index = world
        .world_items()
        .items
        .iter()
        .position(|row| row.presence == presence)?;
    let row = world.world_items_mut().items.remove(index);
    if let Some(number) = world.gentity_number(presence) {
        world.remove_script_mover_by_number(number);
    }
    world.remove_collision_owner(presence);
    Some(row.item)
}

pub(crate) fn clear(world: &mut FrameWorld) {
    let presences: Vec<_> = world
        .world_items()
        .items
        .iter()
        .map(|row| row.presence)
        .collect();
    for presence in presences {
        remove(world, presence);
    }
}

/// Rebuilds model rows for adopted stacks. Adoption restores movers but not
/// collision owners.
pub(crate) fn restore_owners(world: &mut SimState) {
    let rows = world.world_items().items.clone();
    let stale: Vec<_> = world
        .entity_collision_capabilities()
        .iter()
        .filter_map(|row| row.owner.script_model())
        .filter(|id| id.to_wire() >= WORLD_ITEM_PRESENCE_BASE)
        .filter(|id| !rows.iter().any(|row| row.presence == *id))
        .collect();
    for id in stale {
        world.remove_collision_owner(id);
    }
    for row in &rows {
        if world.collision_owner_mut(row.presence).is_none() {
            insert_owner(world, row);
        }
    }
}
