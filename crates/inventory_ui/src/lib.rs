mod model;
mod runtime;
mod view;

pub use model::{
    DropAction, DropId, DropRequest, DropResult, InventoryOverlay, InventoryOverlayRequest,
    InventoryPane, InventoryView, PendingDrop, apply_action,
};
pub use runtime::InventoryUiPlugin;

#[derive(bevy::prelude::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryUiSet {
    Input,
    Adapter,
    Present,
}
