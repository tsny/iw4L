use crate::bullet_collision::MASK_PLAYER_SOLID;
use crate::frame::FrameWorld;
use crate::item::{ITEM_MAXS, ITEM_MINS, WEAP_INVENTORY_PRIMARY};
use crate::{ClientId, ClientLifecycle, EntityRef};

pub const DEBUG_SPAWN_MAX_DISTANCE: f32 = 4096.0;
const DEBUG_SPAWN_MIN_NORMAL_Z: f32 = 0.7;
const DEBUG_SPAWN_SURFACE_OFFSET: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugSpawnRejectReason {
    NotAllowed,
    NotAlive,
    NoWorld,
    NoSurface,
    SurfaceTooSteep,
    Blocked,
    UnsupportedWeapon,
    CapacityExhausted,
}

impl DebugSpawnRejectReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAllowed => "debug spawning is not allowed",
            Self::NotAlive => "player is not alive",
            Self::NoWorld => "world collision is unavailable",
            Self::NoSurface => "the view ray did not hit a surface",
            Self::SurfaceTooSteep => "the selected surface is too steep",
            Self::Blocked => "the selected point is blocked",
            Self::UnsupportedWeapon => "weapon cannot be a world pickup",
            Self::CapacityExhausted => "world entity capacity is exhausted",
        }
    }

    pub const fn wire_tag(self) -> u8 {
        match self {
            Self::NotAllowed => 0,
            Self::NotAlive => 1,
            Self::NoWorld => 2,
            Self::NoSurface => 3,
            Self::SurfaceTooSteep => 4,
            Self::Blocked => 5,
            Self::UnsupportedWeapon => 6,
            Self::CapacityExhausted => 7,
        }
    }

    pub const fn from_wire_tag(tag: u8) -> Option<Self> {
        Some(match tag {
            0 => Self::NotAllowed,
            1 => Self::NotAlive,
            2 => Self::NoWorld,
            3 => Self::NoSurface,
            4 => Self::SurfaceTooSteep,
            5 => Self::Blocked,
            6 => Self::UnsupportedWeapon,
            7 => Self::CapacityExhausted,
            _ => return None,
        })
    }
}

impl core::fmt::Display for DebugSpawnRejectReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugSpawnPlacement {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub yaw: f32,
}

pub(crate) fn resolve_placement(
    world: &FrameWorld,
    requester: ClientId,
    mins: [f32; 3],
    maxs: [f32; 3],
) -> Result<DebugSpawnPlacement, DebugSpawnRejectReason> {
    if !world
        .client_meta(requester)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive)
    {
        return Err(DebugSpawnRejectReason::NotAlive);
    }
    if !world.has_world_clip() {
        return Err(DebugSpawnRejectReason::NoWorld);
    }
    let ps = world
        .player(requester)
        .ok_or(DebugSpawnRejectReason::NotAlive)?;
    let eye = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    let forward = math_iw4::angle_vectors(ps.viewangles).0;
    let end = core::array::from_fn(|axis| eye[axis] + forward[axis] * DEBUG_SPAWN_MAX_DISTANCE);
    let hit = world.trace_static_world(eye, end, [0.0; 3], [0.0; 3], MASK_PLAYER_SOLID);
    if hit.startsolid != 0 || hit.allsolid != 0 {
        return Err(DebugSpawnRejectReason::Blocked);
    }
    if hit.fraction >= 1.0 {
        return Err(DebugSpawnRejectReason::NoSurface);
    }
    if hit.normal[2] < DEBUG_SPAWN_MIN_NORMAL_Z {
        return Err(DebugSpawnRejectReason::SurfaceTooSteep);
    }
    let origin = core::array::from_fn(|axis| {
        hit.endpos[axis] + hit.normal[axis] * DEBUG_SPAWN_SURFACE_OFFSET
    });
    if !origin.iter().all(|value| value.is_finite()) {
        return Err(DebugSpawnRejectReason::Blocked);
    }
    let clearance = world.trace_world(origin, origin, mins, maxs, MASK_PLAYER_SOLID);
    if clearance.startsolid != 0 || clearance.allsolid != 0 {
        return Err(DebugSpawnRejectReason::Blocked);
    }
    Ok(DebugSpawnPlacement {
        origin,
        normal: hit.normal,
        yaw: ps.viewangles[1],
    })
}

pub(crate) fn spawn_weapon(
    world: &mut FrameWorld,
    requester: ClientId,
    weapon: u32,
) -> Result<(EntityRef, DebugSpawnPlacement), DebugSpawnRejectReason> {
    let facts = world
        .combat_facts_for(weapon)
        .filter(|facts| weapon != 0 && facts.inventory_type == WEAP_INVENTORY_PRIMARY)
        .ok_or(DebugSpawnRejectReason::UnsupportedWeapon)?;
    let (clip_r, clip_l, stock) = weapon_iw4::spawn_clip_stock(&facts, i32::from(facts.dual_wield));
    let placement = resolve_placement(world, requester, ITEM_MINS, ITEM_MAXS)?;
    let entity = crate::item::spawn_stationary_weapon(
        world,
        weapon,
        placement.origin,
        placement.yaw,
        clip_r,
        clip_l,
        stock,
    )
    .ok_or(DebugSpawnRejectReason::CapacityExhausted)?;
    Ok((entity, placement))
}
