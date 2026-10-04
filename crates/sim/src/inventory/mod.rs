use std::sync::OnceLock;

use ::inventory::{
    Catalog, ContainerId, GridContainer, GridError, ItemDefId, ItemInstance, ItemInstanceId,
    MAX_CONDITION, Placement,
};

use crate::{ActionRequestId, ClientId};

const BASE_ITEMS: &str = include_str!("../../../../content/loot/base/items.json");

pub const BACKPACK_WIDTH: u8 = 8;
pub const BACKPACK_HEIGHT: u8 = 6;

pub fn loot_catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::from_json(BASE_ITEMS).expect("embedded base loot catalog must remain valid")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryGrantRejectReason {
    NotAllowed,
    NotAlive,
    InvalidKey,
    UnknownItem,
    InvalidQuantity,
    NoSpace,
    InvalidState,
    InstanceIdsExhausted,
}

impl InventoryGrantRejectReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAllowed => "debug inventory actions are disabled",
            Self::NotAlive => "player is not alive",
            Self::InvalidKey => "item key is invalid",
            Self::UnknownItem => "item key is not in the catalog",
            Self::InvalidQuantity => "quantity is outside the supported range",
            Self::NoSpace => "backpack has no valid placement",
            Self::InvalidState => "backpack state is invalid",
            Self::InstanceIdsExhausted => "item instance IDs are exhausted",
        }
    }

    pub const fn wire_tag(self) -> u8 {
        match self {
            Self::NotAllowed => 0,
            Self::NotAlive => 1,
            Self::InvalidKey => 2,
            Self::UnknownItem => 3,
            Self::InvalidQuantity => 4,
            Self::NoSpace => 5,
            Self::InvalidState => 6,
            Self::InstanceIdsExhausted => 7,
        }
    }

    pub const fn from_wire_tag(tag: u8) -> Option<Self> {
        Some(match tag {
            0 => Self::NotAllowed,
            1 => Self::NotAlive,
            2 => Self::InvalidKey,
            3 => Self::UnknownItem,
            4 => Self::InvalidQuantity,
            5 => Self::NoSpace,
            6 => Self::InvalidState,
            7 => Self::InstanceIdsExhausted,
            _ => return None,
        })
    }
}

impl core::fmt::Display for InventoryGrantRejectReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryNotice {
    pub request_id: ActionRequestId,
    pub definition: ItemDefId,
    pub quantity: u16,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventorySummary {
    pub weight_g: u64,
    pub used_cells: u16,
    pub total_cells: u16,
    pub instances: u16,
    pub revision: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerInventory {
    backpack: GridContainer,
    next_serial: u32,
    latest_notice: Option<InventoryNotice>,
}

impl PlayerInventory {
    pub fn new(owner: ClientId) -> Self {
        Self {
            backpack: GridContainer::new(
                backpack_container_id(owner),
                BACKPACK_WIDTH,
                BACKPACK_HEIGHT,
            )
            .expect("fixed backpack dimensions are valid"),
            next_serial: 1,
            latest_notice: None,
        }
    }

    pub fn from_rows(
        owner: ClientId,
        revision: u32,
        next_serial: u32,
        items: Vec<ItemInstance>,
        placements: Vec<Placement>,
        latest_notice: Option<InventoryNotice>,
    ) -> Result<Self, GridError> {
        let inventory = Self {
            backpack: GridContainer::from_rows(
                backpack_container_id(owner),
                BACKPACK_WIDTH,
                BACKPACK_HEIGHT,
                revision,
                items,
                placements,
                loot_catalog(),
            )?,
            next_serial,
            latest_notice,
        };
        inventory
            .check_owner(owner)
            .map_err(|_| GridError::ContainerInvariant(inventory.backpack.id()))?;
        Ok(inventory)
    }

    pub fn backpack(&self) -> &GridContainer {
        &self.backpack
    }

    pub fn next_serial(&self) -> u32 {
        self.next_serial
    }

    pub fn latest_notice(&self) -> Option<InventoryNotice> {
        self.latest_notice
    }

    pub fn summary(&self) -> Result<InventorySummary, GridError> {
        let catalog = loot_catalog();
        self.backpack.check_invariants(catalog)?;
        let mut used_cells = 0u16;
        for item in self.backpack.items() {
            let definition = catalog
                .definition(item.definition)
                .ok_or(GridError::UnknownDefinition(item.definition))?;
            used_cells = used_cells.saturating_add(
                u16::from(definition.footprint.width) * u16::from(definition.footprint.height),
            );
        }
        Ok(InventorySummary {
            weight_g: self.backpack.total_weight_g(catalog)?,
            used_cells,
            total_cells: u16::from(self.backpack.width()) * u16::from(self.backpack.height()),
            instances: self.backpack.items().len().try_into().unwrap_or(u16::MAX),
            revision: self.backpack.revision(),
        })
    }

    pub fn grant(
        &mut self,
        owner: ClientId,
        request_id: ActionRequestId,
        definition: ItemDefId,
        quantity: u16,
    ) -> Result<InventoryNotice, InventoryGrantRejectReason> {
        if quantity == 0 {
            return Err(InventoryGrantRejectReason::InvalidQuantity);
        }
        self.check_owner(owner)?;
        let catalog = loot_catalog();
        let definition_row = catalog
            .definition(definition)
            .ok_or(InventoryGrantRejectReason::UnknownItem)?;
        let mut next = self.clone();
        let mut remaining = quantity;
        let stack_ids: Vec<_> = next
            .backpack
            .items()
            .iter()
            .filter(|item| item.definition == definition && item.condition == MAX_CONDITION)
            .map(|item| item.id)
            .collect();
        for instance in stack_ids {
            if remaining == 0 {
                break;
            }
            let current = next
                .backpack
                .item(instance)
                .ok_or(InventoryGrantRejectReason::InvalidState)?
                .quantity;
            let added = remaining.min(definition_row.max_stack - current);
            if added == 0 {
                continue;
            }
            next.backpack
                .increase_stack(instance, next.backpack.revision(), added, catalog)
                .map_err(|_| InventoryGrantRejectReason::InvalidState)?;
            remaining -= added;
        }
        while remaining != 0 {
            let stack = remaining.min(definition_row.max_stack);
            let instance = next.allocate_instance(owner)?;
            next.backpack
                .insert_first_fit(
                    ItemInstance {
                        id: instance,
                        definition,
                        quantity: stack,
                        condition: MAX_CONDITION,
                    },
                    catalog,
                )
                .map_err(|error| match error {
                    GridError::NoPlacement | GridError::TooManyItems(_) => {
                        InventoryGrantRejectReason::NoSpace
                    }
                    _ => InventoryGrantRejectReason::InvalidState,
                })?;
            remaining -= stack;
        }
        let notice = InventoryNotice {
            request_id,
            definition,
            quantity,
            revision: next.backpack.revision(),
        };
        next.latest_notice = Some(notice);
        *self = next;
        Ok(notice)
    }

    pub fn check_owner(&self, owner: ClientId) -> Result<(), InventoryGrantRejectReason> {
        if self.backpack.id() != backpack_container_id(owner) || self.next_serial == 0 {
            return Err(InventoryGrantRejectReason::InvalidState);
        }
        if self.latest_notice.is_some_and(|notice| {
            notice.quantity == 0
                || notice.revision > self.backpack.revision()
                || loot_catalog().definition(notice.definition).is_none()
        }) {
            return Err(InventoryGrantRejectReason::InvalidState);
        }
        let owner_prefix = u64::from(owner.0) << 32;
        let mut highest = 0u32;
        for item in self.backpack.items() {
            if item.id.0 >> 32 != u64::from(owner.0) {
                return Err(InventoryGrantRejectReason::InvalidState);
            }
            let serial = item.id.0 as u32;
            if serial == 0 {
                return Err(InventoryGrantRejectReason::InvalidState);
            }
            highest = highest.max(serial);
            if item.id.0 != owner_prefix | u64::from(serial) {
                return Err(InventoryGrantRejectReason::InvalidState);
            }
        }
        if self.next_serial <= highest {
            return Err(InventoryGrantRejectReason::InvalidState);
        }
        self.backpack
            .check_invariants(loot_catalog())
            .map_err(|_| InventoryGrantRejectReason::InvalidState)
    }

    fn allocate_instance(
        &mut self,
        owner: ClientId,
    ) -> Result<ItemInstanceId, InventoryGrantRejectReason> {
        let serial = self.next_serial;
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .ok_or(InventoryGrantRejectReason::InstanceIdsExhausted)?;
        Ok(ItemInstanceId(
            (u64::from(owner.0) << 32) | u64::from(serial),
        ))
    }
}

fn backpack_container_id(owner: ClientId) -> ContainerId {
    ContainerId(owner.0.saturating_add(1).max(1))
}
