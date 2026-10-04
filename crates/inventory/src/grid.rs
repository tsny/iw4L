use std::collections::BTreeSet;

use crate::{Catalog, Footprint, ItemDefId, ItemDefinition};

pub const MAX_CONTAINER_DIMENSION: u8 = 16;
pub const MAX_CONTAINER_ITEMS: usize = 256;
pub const MAX_CONDITION: u16 = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainerId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemInstanceId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemInstance {
    pub id: ItemInstanceId,
    pub definition: ItemDefId,
    pub quantity: u16,
    pub condition: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub instance: ItemInstanceId,
    pub x: u8,
    pub y: u8,
    pub rotated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementTarget {
    pub x: u8,
    pub y: u8,
    pub rotated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferAmount {
    Whole,
    Split {
        quantity: u16,
        new_instance: ItemInstanceId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferRequest {
    pub expected_source_revision: u32,
    pub expected_destination_revision: u32,
    pub instance: ItemInstanceId,
    pub amount: TransferAmount,
    pub target: PlacementTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeRequest {
    pub expected_source_revision: u32,
    pub expected_destination_revision: u32,
    pub source_instance: ItemInstanceId,
    pub destination_instance: ItemInstanceId,
    pub quantity: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridContainer {
    id: ContainerId,
    width: u8,
    height: u8,
    revision: u32,
    items: Vec<ItemInstance>,
    placements: Vec<Placement>,
}

impl GridContainer {
    pub fn new(id: ContainerId, width: u8, height: u8) -> Result<Self, GridError> {
        validate_container_header(id, width, height)?;
        Ok(Self {
            id,
            width,
            height,
            revision: 0,
            items: Vec::new(),
            placements: Vec::new(),
        })
    }

    pub fn from_rows(
        id: ContainerId,
        width: u8,
        height: u8,
        revision: u32,
        mut items: Vec<ItemInstance>,
        mut placements: Vec<Placement>,
        catalog: &Catalog,
    ) -> Result<Self, GridError> {
        validate_container_header(id, width, height)?;
        items.sort_by_key(|item| item.id);
        placements.sort_by_key(|placement| placement.instance);
        let container = Self {
            id,
            width,
            height,
            revision,
            items,
            placements,
        };
        container.check_invariants(catalog)?;
        Ok(container)
    }

    pub fn id(&self) -> ContainerId {
        self.id
    }

    pub fn width(&self) -> u8 {
        self.width
    }

    pub fn height(&self) -> u8 {
        self.height
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn items(&self) -> &[ItemInstance] {
        &self.items
    }

    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    pub fn item(&self, id: ItemInstanceId) -> Option<&ItemInstance> {
        self.items
            .binary_search_by_key(&id, |item| item.id)
            .ok()
            .map(|index| &self.items[index])
    }

    pub fn placement(&self, id: ItemInstanceId) -> Option<Placement> {
        self.placements
            .binary_search_by_key(&id, |placement| placement.instance)
            .ok()
            .map(|index| self.placements[index])
    }

    pub fn expect_revision(&self, expected: u32) -> Result<(), GridError> {
        if self.revision != expected {
            return Err(GridError::StaleRevision {
                container: self.id,
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    pub fn insert(
        &mut self,
        item: ItemInstance,
        target: PlacementTarget,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        self.ensure_can_increment_revision()?;
        self.ensure_insertable(&item, target, catalog)?;
        self.insert_unchecked(item, target);
        self.increment_revision_unchecked();
        Ok(())
    }

    pub fn insert_first_fit(
        &mut self,
        item: ItemInstance,
        catalog: &Catalog,
    ) -> Result<PlacementTarget, GridError> {
        validate_instance(&item, catalog)?;
        if self.item(item.id).is_some() {
            return Err(GridError::DuplicateInstance(item.id));
        }
        let target = self
            .first_fit(item.definition, catalog)?
            .ok_or(GridError::NoPlacement)?;
        self.insert(item, target, catalog)?;
        Ok(target)
    }

    pub fn remove(&mut self, id: ItemInstanceId) -> Result<ItemInstance, GridError> {
        self.ensure_can_increment_revision()?;
        let item = self.remove_unchecked(id)?;
        self.increment_revision_unchecked();
        Ok(item)
    }

    pub fn move_within(
        &mut self,
        id: ItemInstanceId,
        expected_revision: u32,
        target: PlacementTarget,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        self.expect_revision(expected_revision)?;
        self.ensure_can_increment_revision()?;
        let item = self.item(id).ok_or(GridError::ItemNotFound(id))?;
        let definition = definition_for(catalog, item.definition)?;
        self.ensure_placement(definition, target, Some(id), catalog)?;
        let index = self
            .placements
            .binary_search_by_key(&id, |placement| placement.instance)
            .map_err(|_| GridError::PlacementMissing(id))?;
        self.placements[index] = Placement {
            instance: id,
            x: target.x,
            y: target.y,
            rotated: target.rotated,
        };
        self.increment_revision_unchecked();
        Ok(())
    }

    pub fn split_within(
        &mut self,
        source: ItemInstanceId,
        expected_revision: u32,
        quantity: u16,
        new_instance: ItemInstanceId,
        target: PlacementTarget,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        self.expect_revision(expected_revision)?;
        self.ensure_can_increment_revision()?;
        validate_instance_id(new_instance)?;
        if self.item(new_instance).is_some() {
            return Err(GridError::DuplicateInstance(new_instance));
        }
        if self.items.len() >= MAX_CONTAINER_ITEMS {
            return Err(GridError::TooManyItems(self.items.len() + 1));
        }
        let source_index = self
            .items
            .binary_search_by_key(&source, |item| item.id)
            .map_err(|_| GridError::ItemNotFound(source))?;
        let source_item = self.items[source_index].clone();
        if quantity == 0 || quantity >= source_item.quantity {
            return Err(GridError::InvalidSplitQuantity {
                available: source_item.quantity,
                requested: quantity,
            });
        }
        let definition = definition_for(catalog, source_item.definition)?;
        self.ensure_placement(definition, target, None, catalog)?;
        self.items[source_index].quantity -= quantity;
        self.insert_unchecked(
            ItemInstance {
                id: new_instance,
                definition: source_item.definition,
                quantity,
                condition: source_item.condition,
            },
            target,
        );
        self.increment_revision_unchecked();
        Ok(())
    }

    pub fn merge_within(
        &mut self,
        source: ItemInstanceId,
        destination: ItemInstanceId,
        expected_revision: u32,
        quantity: u16,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        self.expect_revision(expected_revision)?;
        self.ensure_can_increment_revision()?;
        if source == destination {
            return Err(GridError::SameInstance(source));
        }
        let source_index = self
            .items
            .binary_search_by_key(&source, |item| item.id)
            .map_err(|_| GridError::ItemNotFound(source))?;
        let destination_index = self
            .items
            .binary_search_by_key(&destination, |item| item.id)
            .map_err(|_| GridError::ItemNotFound(destination))?;
        let source_item = self.items[source_index].clone();
        let destination_item = self.items[destination_index].clone();
        validate_merge(&source_item, &destination_item, quantity, catalog)?;
        self.apply_merge_unchecked(source, destination, quantity);
        self.increment_revision_unchecked();
        Ok(())
    }

    pub fn first_fit(
        &self,
        definition: ItemDefId,
        catalog: &Catalog,
    ) -> Result<Option<PlacementTarget>, GridError> {
        let definition = definition_for(catalog, definition)?;
        for rotated in [false, true] {
            if rotated && !definition.rotatable {
                continue;
            }
            for y in 0..self.height {
                for x in 0..self.width {
                    let target = PlacementTarget { x, y, rotated };
                    match self.ensure_placement(definition, target, None, catalog) {
                        Ok(()) => return Ok(Some(target)),
                        Err(
                            GridError::PlacementOutOfBounds { .. }
                            | GridError::PlacementOverlap { .. }
                            | GridError::NotRotatable(_),
                        ) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn total_weight_g(&self, catalog: &Catalog) -> Result<u64, GridError> {
        let mut total = 0u64;
        for item in &self.items {
            let definition = definition_for(catalog, item.definition)?;
            total = total
                .checked_add(u64::from(definition.weight_g) * u64::from(item.quantity))
                .ok_or(GridError::WeightOverflow)?;
        }
        Ok(total)
    }

    pub fn check_invariants(&self, catalog: &Catalog) -> Result<(), GridError> {
        validate_container_header(self.id, self.width, self.height)?;
        if self.items.len() > MAX_CONTAINER_ITEMS
            || self.items.len() != self.placements.len()
            || !strictly_ordered(self.items.iter().map(|item| item.id))
            || !strictly_ordered(self.placements.iter().map(|placement| placement.instance))
        {
            return Err(GridError::ContainerInvariant(self.id));
        }
        let item_ids: BTreeSet<_> = self.items.iter().map(|item| item.id).collect();
        let placement_ids: BTreeSet<_> = self
            .placements
            .iter()
            .map(|placement| placement.instance)
            .collect();
        if item_ids.len() != self.items.len() || item_ids != placement_ids {
            return Err(GridError::ContainerInvariant(self.id));
        }
        for item in &self.items {
            validate_instance(item, catalog)?;
            let placement = self
                .placement(item.id)
                .ok_or(GridError::PlacementMissing(item.id))?;
            let definition = definition_for(catalog, item.definition)?;
            validate_target_bounds(self.width, self.height, definition, placement.into())?;
        }
        for (index, left) in self.placements.iter().enumerate() {
            let left_item = self
                .item(left.instance)
                .ok_or(GridError::ItemNotFound(left.instance))?;
            let left_definition = definition_for(catalog, left_item.definition)?;
            for right in &self.placements[index + 1..] {
                let right_item = self
                    .item(right.instance)
                    .ok_or(GridError::ItemNotFound(right.instance))?;
                let right_definition = definition_for(catalog, right_item.definition)?;
                if placements_overlap(left, left_definition, right, right_definition) {
                    return Err(GridError::PlacementOverlap {
                        instance: left.instance,
                        with: right.instance,
                    });
                }
            }
        }
        Ok(())
    }

    fn ensure_insertable(
        &self,
        item: &ItemInstance,
        target: PlacementTarget,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        validate_instance(item, catalog)?;
        if self.items.len() >= MAX_CONTAINER_ITEMS {
            return Err(GridError::TooManyItems(self.items.len() + 1));
        }
        if self.item(item.id).is_some() {
            return Err(GridError::DuplicateInstance(item.id));
        }
        let definition = definition_for(catalog, item.definition)?;
        self.ensure_placement(definition, target, None, catalog)
    }

    fn ensure_placement(
        &self,
        definition: &ItemDefinition,
        target: PlacementTarget,
        ignore: Option<ItemInstanceId>,
        catalog: &Catalog,
    ) -> Result<(), GridError> {
        validate_target_bounds(self.width, self.height, definition, target)?;
        let candidate = Placement {
            instance: ignore.unwrap_or(ItemInstanceId(0)),
            x: target.x,
            y: target.y,
            rotated: target.rotated,
        };
        for placement in &self.placements {
            if Some(placement.instance) == ignore {
                continue;
            }
            let item = self
                .item(placement.instance)
                .ok_or(GridError::ItemNotFound(placement.instance))?;
            let occupied_definition = definition_for(catalog, item.definition)?;
            if placements_overlap(&candidate, definition, placement, occupied_definition) {
                return Err(GridError::PlacementOverlap {
                    instance: candidate.instance,
                    with: placement.instance,
                });
            }
        }
        Ok(())
    }

    fn insert_unchecked(&mut self, item: ItemInstance, target: PlacementTarget) {
        self.items.push(item.clone());
        self.placements.push(Placement {
            instance: item.id,
            x: target.x,
            y: target.y,
            rotated: target.rotated,
        });
        self.items.sort_by_key(|row| row.id);
        self.placements.sort_by_key(|row| row.instance);
    }

    fn remove_unchecked(&mut self, id: ItemInstanceId) -> Result<ItemInstance, GridError> {
        let item_index = self
            .items
            .binary_search_by_key(&id, |item| item.id)
            .map_err(|_| GridError::ItemNotFound(id))?;
        let placement_index = self
            .placements
            .binary_search_by_key(&id, |placement| placement.instance)
            .map_err(|_| GridError::PlacementMissing(id))?;
        self.placements.remove(placement_index);
        Ok(self.items.remove(item_index))
    }

    fn apply_merge_unchecked(
        &mut self,
        source: ItemInstanceId,
        destination: ItemInstanceId,
        quantity: u16,
    ) {
        let source_index = self
            .items
            .binary_search_by_key(&source, |item| item.id)
            .expect("validated source");
        let destination_index = self
            .items
            .binary_search_by_key(&destination, |item| item.id)
            .expect("validated destination");
        self.items[destination_index].quantity += quantity;
        if self.items[source_index].quantity == quantity {
            self.remove_unchecked(source)
                .expect("validated source rows");
        } else {
            self.items[source_index].quantity -= quantity;
        }
    }

    fn ensure_can_increment_revision(&self) -> Result<(), GridError> {
        if self.revision == u32::MAX {
            return Err(GridError::RevisionOverflow(self.id));
        }
        Ok(())
    }

    fn increment_revision_unchecked(&mut self) {
        self.revision += 1;
    }
}

impl From<Placement> for PlacementTarget {
    fn from(value: Placement) -> Self {
        Self {
            x: value.x,
            y: value.y,
            rotated: value.rotated,
        }
    }
}

pub fn transfer_between(
    source: &mut GridContainer,
    destination: &mut GridContainer,
    request: TransferRequest,
    catalog: &Catalog,
) -> Result<ItemInstanceId, GridError> {
    if source.id == destination.id {
        return Err(GridError::SameContainer(source.id));
    }
    source.expect_revision(request.expected_source_revision)?;
    destination.expect_revision(request.expected_destination_revision)?;
    source.ensure_can_increment_revision()?;
    destination.ensure_can_increment_revision()?;
    let source_item = source
        .item(request.instance)
        .cloned()
        .ok_or(GridError::ItemNotFound(request.instance))?;
    let moved = match request.amount {
        TransferAmount::Whole => source_item.clone(),
        TransferAmount::Split {
            quantity,
            new_instance,
        } => {
            validate_instance_id(new_instance)?;
            if quantity == 0 || quantity >= source_item.quantity {
                return Err(GridError::InvalidSplitQuantity {
                    available: source_item.quantity,
                    requested: quantity,
                });
            }
            if source.item(new_instance).is_some() || destination.item(new_instance).is_some() {
                return Err(GridError::DuplicateInstance(new_instance));
            }
            ItemInstance {
                id: new_instance,
                definition: source_item.definition,
                quantity,
                condition: source_item.condition,
            }
        }
    };
    destination.ensure_insertable(&moved, request.target, catalog)?;

    match request.amount {
        TransferAmount::Whole => {
            source.remove_unchecked(source_item.id)?;
        }
        TransferAmount::Split { quantity, .. } => {
            let index = source
                .items
                .binary_search_by_key(&source_item.id, |item| item.id)
                .expect("validated source item");
            source.items[index].quantity -= quantity;
        }
    }
    let moved_id = moved.id;
    destination.insert_unchecked(moved, request.target);
    source.increment_revision_unchecked();
    destination.increment_revision_unchecked();
    Ok(moved_id)
}

pub fn merge_between(
    source: &mut GridContainer,
    destination: &mut GridContainer,
    request: MergeRequest,
    catalog: &Catalog,
) -> Result<(), GridError> {
    if source.id == destination.id {
        return Err(GridError::SameContainer(source.id));
    }
    if request.source_instance == request.destination_instance {
        return Err(GridError::SameInstance(request.source_instance));
    }
    source.expect_revision(request.expected_source_revision)?;
    destination.expect_revision(request.expected_destination_revision)?;
    source.ensure_can_increment_revision()?;
    destination.ensure_can_increment_revision()?;
    let source_item = source
        .item(request.source_instance)
        .cloned()
        .ok_or(GridError::ItemNotFound(request.source_instance))?;
    let destination_item = destination
        .item(request.destination_instance)
        .cloned()
        .ok_or(GridError::ItemNotFound(request.destination_instance))?;
    validate_merge(&source_item, &destination_item, request.quantity, catalog)?;

    let destination_index = destination
        .items
        .binary_search_by_key(&destination_item.id, |item| item.id)
        .expect("validated destination item");
    destination.items[destination_index].quantity += request.quantity;
    if source_item.quantity == request.quantity {
        source.remove_unchecked(source_item.id)?;
    } else {
        let source_index = source
            .items
            .binary_search_by_key(&source_item.id, |item| item.id)
            .expect("validated source item");
        source.items[source_index].quantity -= request.quantity;
    }
    source.increment_revision_unchecked();
    destination.increment_revision_unchecked();
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridError {
    InvalidContainerId,
    InvalidContainerDimensions([u8; 2]),
    TooManyItems(usize),
    InvalidInstanceId,
    DuplicateInstance(ItemInstanceId),
    UnknownDefinition(ItemDefId),
    InvalidQuantity {
        definition: ItemDefId,
        quantity: u16,
        maximum: u16,
    },
    InvalidCondition(u16),
    ItemNotFound(ItemInstanceId),
    PlacementMissing(ItemInstanceId),
    PlacementOutOfBounds {
        x: u8,
        y: u8,
        width: u8,
        height: u8,
    },
    PlacementOverlap {
        instance: ItemInstanceId,
        with: ItemInstanceId,
    },
    NotRotatable(ItemDefId),
    NoPlacement,
    StaleRevision {
        container: ContainerId,
        expected: u32,
        actual: u32,
    },
    RevisionOverflow(ContainerId),
    SameContainer(ContainerId),
    SameInstance(ItemInstanceId),
    InvalidSplitQuantity {
        available: u16,
        requested: u16,
    },
    DefinitionMismatch,
    ConditionMismatch,
    StackCapacity {
        available: u16,
        requested: u16,
    },
    WeightOverflow,
    ContainerInvariant(ContainerId),
}

impl core::fmt::Display for GridError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidContainerId => f.write_str("container ID zero is reserved"),
            Self::InvalidContainerDimensions([width, height]) => {
                write!(f, "invalid container dimensions {width}x{height}")
            }
            Self::TooManyItems(count) => write!(f, "container item count {count} exceeds limit"),
            Self::InvalidInstanceId => f.write_str("item instance ID zero is reserved"),
            Self::DuplicateInstance(id) => write!(f, "duplicate item instance {}", id.0),
            Self::UnknownDefinition(id) => write!(f, "unknown item definition {}", id.0),
            Self::InvalidQuantity {
                definition,
                quantity,
                maximum,
            } => write!(
                f,
                "item definition {} quantity {quantity} exceeds 1..={maximum}",
                definition.0
            ),
            Self::InvalidCondition(value) => {
                write!(f, "item condition {value} exceeds {MAX_CONDITION}")
            }
            Self::ItemNotFound(id) => write!(f, "item instance {} not found", id.0),
            Self::PlacementMissing(id) => write!(f, "item instance {} has no placement", id.0),
            Self::PlacementOutOfBounds {
                x,
                y,
                width,
                height,
            } => write!(
                f,
                "placement at {x},{y} with size {width}x{height} is out of bounds"
            ),
            Self::PlacementOverlap { instance, with } => write!(
                f,
                "item instance {} overlaps item instance {}",
                instance.0, with.0
            ),
            Self::NotRotatable(id) => write!(f, "item definition {} cannot rotate", id.0),
            Self::NoPlacement => f.write_str("container has no valid placement"),
            Self::StaleRevision {
                container,
                expected,
                actual,
            } => write!(
                f,
                "container {} revision is {actual}, expected {expected}",
                container.0
            ),
            Self::RevisionOverflow(id) => write!(f, "container {} revision overflow", id.0),
            Self::SameContainer(id) => write!(f, "container {} cannot transfer to itself", id.0),
            Self::SameInstance(id) => write!(f, "item instance {} cannot merge with itself", id.0),
            Self::InvalidSplitQuantity {
                available,
                requested,
            } => write!(
                f,
                "cannot split quantity {requested} from available {available}"
            ),
            Self::DefinitionMismatch => f.write_str("stack item definitions differ"),
            Self::ConditionMismatch => f.write_str("stack item conditions differ"),
            Self::StackCapacity {
                available,
                requested,
            } => write!(
                f,
                "stack has capacity {available}, requested quantity {requested}"
            ),
            Self::WeightOverflow => f.write_str("container weight overflow"),
            Self::ContainerInvariant(id) => write!(f, "container {} invariant failed", id.0),
        }
    }
}

impl std::error::Error for GridError {}

fn validate_container_header(id: ContainerId, width: u8, height: u8) -> Result<(), GridError> {
    if id.0 == 0 {
        return Err(GridError::InvalidContainerId);
    }
    if width == 0
        || height == 0
        || width > MAX_CONTAINER_DIMENSION
        || height > MAX_CONTAINER_DIMENSION
    {
        return Err(GridError::InvalidContainerDimensions([width, height]));
    }
    Ok(())
}

fn validate_instance_id(id: ItemInstanceId) -> Result<(), GridError> {
    if id.0 == 0 {
        return Err(GridError::InvalidInstanceId);
    }
    Ok(())
}

fn validate_instance(item: &ItemInstance, catalog: &Catalog) -> Result<(), GridError> {
    validate_instance_id(item.id)?;
    let definition = definition_for(catalog, item.definition)?;
    if item.quantity == 0 || item.quantity > definition.max_stack {
        return Err(GridError::InvalidQuantity {
            definition: item.definition,
            quantity: item.quantity,
            maximum: definition.max_stack,
        });
    }
    if item.condition > MAX_CONDITION {
        return Err(GridError::InvalidCondition(item.condition));
    }
    Ok(())
}

fn definition_for(catalog: &Catalog, id: ItemDefId) -> Result<&ItemDefinition, GridError> {
    catalog
        .definition(id)
        .ok_or(GridError::UnknownDefinition(id))
}

fn validate_target_bounds(
    container_width: u8,
    container_height: u8,
    definition: &ItemDefinition,
    target: PlacementTarget,
) -> Result<Footprint, GridError> {
    if target.rotated && !definition.rotatable {
        return Err(GridError::NotRotatable(definition.id));
    }
    let footprint = definition.footprint.oriented(target.rotated);
    let right = u16::from(target.x) + u16::from(footprint.width);
    let bottom = u16::from(target.y) + u16::from(footprint.height);
    if right > u16::from(container_width) || bottom > u16::from(container_height) {
        return Err(GridError::PlacementOutOfBounds {
            x: target.x,
            y: target.y,
            width: footprint.width,
            height: footprint.height,
        });
    }
    Ok(footprint)
}

fn placements_overlap(
    left: &Placement,
    left_definition: &ItemDefinition,
    right: &Placement,
    right_definition: &ItemDefinition,
) -> bool {
    let left_size = left_definition.footprint.oriented(left.rotated);
    let right_size = right_definition.footprint.oriented(right.rotated);
    let left_right = u16::from(left.x) + u16::from(left_size.width);
    let left_bottom = u16::from(left.y) + u16::from(left_size.height);
    let right_right = u16::from(right.x) + u16::from(right_size.width);
    let right_bottom = u16::from(right.y) + u16::from(right_size.height);
    u16::from(left.x) < right_right
        && u16::from(right.x) < left_right
        && u16::from(left.y) < right_bottom
        && u16::from(right.y) < left_bottom
}

fn validate_merge(
    source: &ItemInstance,
    destination: &ItemInstance,
    quantity: u16,
    catalog: &Catalog,
) -> Result<(), GridError> {
    if source.definition != destination.definition {
        return Err(GridError::DefinitionMismatch);
    }
    if source.condition != destination.condition {
        return Err(GridError::ConditionMismatch);
    }
    if quantity == 0 || quantity > source.quantity {
        return Err(GridError::InvalidSplitQuantity {
            available: source.quantity,
            requested: quantity,
        });
    }
    let definition = definition_for(catalog, source.definition)?;
    let available = definition.max_stack - destination.quantity;
    if quantity > available {
        return Err(GridError::StackCapacity {
            available,
            requested: quantity,
        });
    }
    Ok(())
}

fn strictly_ordered<T: Ord>(values: impl Iterator<Item = T>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|old| old >= &value) {
            return false;
        }
        previous = Some(value);
    }
    true
}
