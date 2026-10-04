use std::collections::BTreeMap;

use bevy::prelude::*;
use inventory::{
    Catalog, ContainerId, GridContainer, ItemInstanceId, PlacementTarget, TransferAmount,
    TransferRequest, transfer_between,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryPane {
    Player,
    Container,
}

impl InventoryPane {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Player => "Backpack",
            Self::Container => "Loot container",
        }
    }
}

#[derive(Resource, Clone)]
pub struct InventoryView {
    pub catalog: Catalog,
    pub player: GridContainer,
    pub container: Option<GridContainer>,
}

impl InventoryView {
    pub fn grid(&self, pane: InventoryPane) -> Option<&GridContainer> {
        match pane {
            InventoryPane::Player => Some(&self.player),
            InventoryPane::Container => self.container.as_ref(),
        }
    }

    pub fn replace(
        &mut self,
        catalog: Catalog,
        player: GridContainer,
        container: Option<GridContainer>,
    ) {
        if self.catalog != catalog || self.player != player || self.container != container {
            self.catalog = catalog;
            self.player = player;
            self.container = container;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DraggedItem {
    pub pane: InventoryPane,
    pub instance: ItemInstanceId,
    pub rotated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoverTarget {
    Cell {
        pane: InventoryPane,
        x: u8,
        y: u8,
    },
    Item {
        pane: InventoryPane,
        instance: ItemInstanceId,
    },
}

#[derive(Resource, Debug)]
pub struct InventoryOverlay {
    pub open: bool,
    pub status: String,
    pub(crate) drag: Option<DraggedItem>,
    pub(crate) hover: Option<HoverTarget>,
    pub(crate) pending: BTreeMap<ItemInstanceId, DropId>,
    next_drop: u64,
}

impl Default for InventoryOverlay {
    fn default() -> Self {
        Self {
            open: false,
            status: "Drag an item to a cell, or off the grid to drop it. R rotates.".into(),
            drag: None,
            hover: None,
            pending: BTreeMap::new(),
            next_drop: 1,
        }
    }
}

impl InventoryOverlay {
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.drag = None;
            self.hover = None;
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn begin_drag(
        &mut self,
        view: &InventoryView,
        pane: InventoryPane,
        instance: ItemInstanceId,
    ) -> Result<(), String> {
        if self.pending.contains_key(&instance) {
            return Err("That item is waiting for authority.".into());
        }
        let placement = view
            .grid(pane)
            .and_then(|grid| grid.placement(instance))
            .ok_or("item is unavailable")?;
        self.drag = Some(DraggedItem {
            pane,
            instance,
            rotated: placement.rotated,
        });
        Ok(())
    }

    pub fn hover_cell(&mut self, pane: InventoryPane, x: u8, y: u8) {
        self.hover = Some(HoverTarget::Cell { pane, x, y });
    }

    pub fn rotate_drag(&mut self, view: &InventoryView) -> Result<(), String> {
        let mut drag = self.drag.ok_or("No item is being dragged.")?;
        let rotatable = view
            .grid(drag.pane)
            .and_then(|grid| grid.item(drag.instance))
            .and_then(|item| view.catalog.definition(item.definition))
            .is_some_and(|definition| definition.rotatable);
        if !rotatable {
            return Err("This item cannot rotate.".into());
        }
        drag.rotated = !drag.rotated;
        self.drag = Some(drag);
        Ok(())
    }

    pub fn complete_drop(&mut self, view: &InventoryView) -> Result<DropRequest, String> {
        let dragged = self.drag.take().ok_or("No item is being dragged.")?;
        let action = match self.hover {
            Some(target) => evaluate_drop(view, dragged, target)?,
            None => evaluate_world_drop(view, dragged)?,
        };
        let id = self.allocate_drop();
        self.pending.insert(dragged.instance, id);
        Ok(DropRequest { id, action })
    }

    pub fn resolve(&mut self, result: &DropResult) {
        self.pending.retain(|_, id| *id != result.id);
        self.status = result.detail.clone();
    }

    pub(crate) fn allocate_drop(&mut self) -> DropId {
        let id = DropId(self.next_drop);
        self.next_drop = self.next_drop.wrapping_add(1).max(1);
        id
    }
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryOverlayRequest {
    Open,
    Close,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DropId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    Place {
        source: InventoryPane,
        destination: InventoryPane,
        source_container: ContainerId,
        destination_container: ContainerId,
        expected_source_revision: u32,
        expected_destination_revision: u32,
        instance: ItemInstanceId,
        target: PlacementTarget,
    },
    Merge {
        pane: InventoryPane,
        container: ContainerId,
        expected_revision: u32,
        source_instance: ItemInstanceId,
        destination_instance: ItemInstanceId,
        quantity: u16,
    },
    DropToWorld {
        container: ContainerId,
        expected_revision: u32,
        instance: ItemInstanceId,
    },
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropRequest {
    pub id: DropId,
    pub action: DropAction,
}

#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct DropResult {
    pub id: DropId,
    pub accepted: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingDrop {
    pub id: DropId,
    pub instance: ItemInstanceId,
}

pub fn evaluate_drop(
    view: &InventoryView,
    dragged: DraggedItem,
    target: HoverTarget,
) -> Result<DropAction, String> {
    let source = view
        .grid(dragged.pane)
        .ok_or("source container is unavailable")?;
    let item = source
        .item(dragged.instance)
        .ok_or("source item is unavailable")?;
    match target {
        HoverTarget::Cell { pane, x, y } => {
            let destination = view
                .grid(pane)
                .ok_or("destination container is unavailable")?;
            let placement = PlacementTarget {
                x,
                y,
                rotated: dragged.rotated,
            };
            let action = DropAction::Place {
                source: dragged.pane,
                destination: pane,
                source_container: source.id(),
                destination_container: destination.id(),
                expected_source_revision: source.revision(),
                expected_destination_revision: destination.revision(),
                instance: dragged.instance,
                target: placement,
            };
            preview_action(view, action)?;
            Ok(action)
        }
        HoverTarget::Item { pane, instance } => {
            if pane != dragged.pane {
                return Err("cross-container stack merging is not available".into());
            }
            if instance == dragged.instance {
                return Err("drop onto a different item or cell".into());
            }
            let action = DropAction::Merge {
                pane,
                container: source.id(),
                expected_revision: source.revision(),
                source_instance: dragged.instance,
                destination_instance: instance,
                quantity: item.quantity,
            };
            preview_action(view, action)?;
            Ok(action)
        }
    }
}

/// Releasing a backpack item off every grid drops it at the player's feet.
pub(crate) fn evaluate_world_drop(
    view: &InventoryView,
    dragged: DraggedItem,
) -> Result<DropAction, String> {
    if dragged.pane != InventoryPane::Player {
        return Err("Only backpack items can be dropped.".into());
    }
    let item = view
        .player
        .item(dragged.instance)
        .ok_or("source item is unavailable")?;
    let definition = view
        .catalog
        .definition(item.definition)
        .ok_or("item definition is unavailable")?;
    if definition.world_model.is_none() {
        return Err(format!(
            "{} has no world model and cannot be dropped.",
            definition.name
        ));
    }
    Ok(DropAction::DropToWorld {
        container: view.player.id(),
        expected_revision: view.player.revision(),
        instance: dragged.instance,
    })
}

pub fn preview_action(view: &InventoryView, action: DropAction) -> Result<(), String> {
    let mut player = view.player.clone();
    let mut container = view.container.clone();
    apply_action(&view.catalog, &mut player, &mut container, action)
}

pub fn apply_action(
    catalog: &Catalog,
    player: &mut GridContainer,
    container: &mut Option<GridContainer>,
    action: DropAction,
) -> Result<(), String> {
    match action {
        DropAction::Place {
            source,
            destination,
            expected_source_revision,
            expected_destination_revision: _,
            instance,
            target,
            ..
        } if source == destination => {
            let grid = pane_mut(player, container, source)?;
            grid.move_within(instance, expected_source_revision, target, catalog)
                .map_err(|error| error.to_string())
        }
        DropAction::Place {
            source: InventoryPane::Player,
            destination: InventoryPane::Container,
            expected_source_revision,
            expected_destination_revision,
            instance,
            target,
            ..
        } => {
            let destination = container
                .as_mut()
                .ok_or("destination container is unavailable")?;
            transfer_between(
                player,
                destination,
                TransferRequest {
                    expected_source_revision,
                    expected_destination_revision,
                    instance,
                    amount: TransferAmount::Whole,
                    target,
                },
                catalog,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        DropAction::Place {
            source: InventoryPane::Container,
            destination: InventoryPane::Player,
            expected_source_revision,
            expected_destination_revision,
            instance,
            target,
            ..
        } => {
            let source = container
                .as_mut()
                .ok_or("source container is unavailable")?;
            transfer_between(
                source,
                player,
                TransferRequest {
                    expected_source_revision,
                    expected_destination_revision,
                    instance,
                    amount: TransferAmount::Whole,
                    target,
                },
                catalog,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        DropAction::Merge {
            pane,
            expected_revision,
            source_instance,
            destination_instance,
            quantity,
            ..
        } => pane_mut(player, container, pane)?
            .merge_within(
                source_instance,
                destination_instance,
                expected_revision,
                quantity,
                catalog,
            )
            .map_err(|error| error.to_string()),
        DropAction::DropToWorld {
            expected_revision,
            instance,
            ..
        } => {
            player
                .expect_revision(expected_revision)
                .map_err(|error| error.to_string())?;
            player
                .remove(instance)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        DropAction::Place { .. } => Err("unsupported container transfer".into()),
    }
}

fn pane_mut<'a>(
    player: &'a mut GridContainer,
    container: &'a mut Option<GridContainer>,
    pane: InventoryPane,
) -> Result<&'a mut GridContainer, String> {
    match pane {
        InventoryPane::Player => Ok(player),
        InventoryPane::Container => container
            .as_mut()
            .ok_or_else(|| "container is unavailable".into()),
    }
}
