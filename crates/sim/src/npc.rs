use crate::bullet_collision::{AuthorityDObjState, CONTENTS_BODY};
use crate::frame::FrameWorld;
use crate::{AuthorityModelOwner, ClientId, EntityCollisionCapabilities, EntityRef, ScriptModelId};

pub const MAX_NPC_ACTORS: usize = 64;
pub const NPC_TARGET_HEALTH: i32 = 100;
pub(crate) const NPC_PRESENCE_BASE: u32 = 0x7100_0000;
const NPC_PRESENCE_END: u32 = 0x7200_0000;
const NPC_SERIAL_END: u32 = NPC_PRESENCE_END - NPC_PRESENCE_BASE;

#[derive(Clone, Debug, PartialEq)]
pub struct NpcActor {
    pub presence: ScriptModelId,
    pub model: [u8; crate::NPC_MODEL_BYTES],
    pub origin: [f32; 3],
    pub yaw: f32,
    pub health: i32,
    pub max_health: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NpcActors {
    pub next_serial: u32,
    pub actors: Vec<NpcActor>,
}

impl Default for NpcActors {
    fn default() -> Self {
        Self {
            next_serial: 1,
            actors: Vec::new(),
        }
    }
}

impl NpcActors {
    pub fn validate(&self) -> bool {
        if self.next_serial == 0
            || self.next_serial > NPC_SERIAL_END
            || self.actors.len() > MAX_NPC_ACTORS
        {
            return false;
        }
        for (index, actor) in self.actors.iter().enumerate() {
            let presence = actor.presence.to_wire();
            let serial = presence.checked_sub(NPC_PRESENCE_BASE);
            if !matches!(serial, Some(serial) if serial > 0 && serial < self.next_serial)
                || crate::npc_model_text(&actor.model).is_none()
                || !actor
                    .origin
                    .iter()
                    .chain([&actor.yaw])
                    .all(|value| value.is_finite())
                || actor.max_health <= 0
                || !(1..=actor.max_health).contains(&actor.health)
                || self.actors[..index]
                    .iter()
                    .any(|other| other.presence == actor.presence)
            {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcSpawnError {
    InvalidModel,
    ModelUnavailable,
    CapacityExhausted,
    NoEntity,
}

pub(crate) fn spawn_target(
    world: &mut FrameWorld,
    model: [u8; crate::NPC_MODEL_BYTES],
    origin: [f32; 3],
    yaw: f32,
) -> Result<EntityRef, NpcSpawnError> {
    let model_name = crate::npc_model_text(&model).ok_or(NpcSpawnError::InvalidModel)?;
    if world.model_capability(model_name).flatten().is_none() {
        return Err(NpcSpawnError::ModelUnavailable);
    }
    if world.npc_actors().actors.len() >= MAX_NPC_ACTORS {
        return Err(NpcSpawnError::CapacityExhausted);
    }
    let serial = world.npc_actors().next_serial;
    if serial >= NPC_SERIAL_END {
        return Err(NpcSpawnError::CapacityExhausted);
    }
    let next_serial = serial
        .checked_add(1)
        .ok_or(NpcSpawnError::CapacityExhausted)?;
    let presence = ScriptModelId::from_wire(
        NPC_PRESENCE_BASE
            .checked_add(serial)
            .ok_or(NpcSpawnError::CapacityExhausted)?,
    );
    let number = world
        .spawn_script_mover(presence, origin, [0.0, yaw, 0.0])
        .map_err(|_| NpcSpawnError::NoEntity)?;
    let entity = match world.entity_kernel().current_ref(number) {
        Ok(entity) => entity,
        Err(_) => {
            world.remove_script_mover_by_number(number);
            return Err(NpcSpawnError::NoEntity);
        }
    };
    let actor = NpcActor {
        presence,
        model,
        origin,
        yaw,
        health: NPC_TARGET_HEALTH,
        max_health: NPC_TARGET_HEALTH,
    };
    insert_owner(world, &actor);
    let actors = world.npc_actors_mut();
    actors.next_serial = next_serial;
    actors.actors.push(actor);
    Ok(entity)
}

fn insert_owner(world: &mut crate::world::SimState, actor: &NpcActor) {
    let Some(model) = crate::npc_model_text(&actor.model) else {
        return;
    };
    let capability = world.model_capability(model).flatten().map(|capability| {
        let mut capability = (*capability).clone();
        capability.contents = Some(CONTENTS_BODY);
        std::sync::Arc::new(capability)
    });
    let angles = [0.0, actor.yaw, 0.0];
    let mut dobj = AuthorityDObjState::at_pose(model, capability, actor.origin, angles);
    dobj.materialize();
    dobj.ensure_bounds_collision();
    let mut owner = EntityCollisionCapabilities::current_tick(
        AuthorityModelOwner::ScriptModel(actor.presence),
        Some(dobj),
        Vec::new(),
    );
    owner.followed_pose = Some((actor.origin, angles));
    world.insert_collision_owner(owner);
}

pub(crate) fn apply_damage(
    world: &mut FrameWorld,
    presence: ScriptModelId,
    amount: i32,
    _attacker: Option<ClientId>,
) -> bool {
    if amount <= 0 {
        return world
            .npc_actors()
            .actors
            .iter()
            .any(|actor| actor.presence == presence);
    }
    let Some(index) = world
        .npc_actors()
        .actors
        .iter()
        .position(|actor| actor.presence == presence)
    else {
        return false;
    };
    let died = {
        let actor = &mut world.npc_actors_mut().actors[index];
        actor.health = actor.health.saturating_sub(amount);
        actor.health <= 0
    };
    if died {
        remove_at(world, index);
    }
    true
}

fn remove_at(world: &mut FrameWorld, index: usize) {
    let actor = world.npc_actors_mut().actors.remove(index);
    if let Some(number) = world.gentity_number(actor.presence) {
        world.remove_script_mover_by_number(number);
    }
    world.remove_collision_owner(actor.presence);
}

pub(crate) fn clear(world: &mut FrameWorld) {
    while !world.npc_actors().actors.is_empty() {
        remove_at(world, 0);
    }
    *world.npc_actors_mut() = NpcActors::default();
}

pub(crate) fn restore_owners(world: &mut crate::world::SimState) {
    let actors = world.npc_actors().actors.clone();
    let stale: Vec<_> = world
        .entity_collision_capabilities()
        .iter()
        .filter_map(|row| row.owner.script_model())
        .filter(|id| (NPC_PRESENCE_BASE..NPC_PRESENCE_END).contains(&id.to_wire()))
        .filter(|id| !actors.iter().any(|actor| actor.presence == *id))
        .collect();
    for id in stale {
        world.remove_collision_owner(id);
    }
    for actor in &actors {
        if world.collision_owner_mut(actor.presence).is_none() {
            insert_owner(world, actor);
        }
    }
}
