use bevy::prelude::*;
use inventory_ui::{DropRequest, DropResult};

use crate::model::LabState;

#[derive(Resource, Default)]
pub struct PendingLabDrops(Vec<DropRequest>);

pub fn fake_transactions(
    mut incoming: MessageReader<DropRequest>,
    mut pending: ResMut<PendingLabDrops>,
    mut results: MessageWriter<DropResult>,
    mut state: ResMut<LabState>,
    mut view: ResMut<inventory_ui::InventoryView>,
) {
    for request in std::mem::take(&mut pending.0) {
        let result = state.apply(request.action);
        results.write(DropResult {
            id: request.id,
            accepted: result.is_ok(),
            detail: result.map_or_else(|error| error, |()| "Inventory updated.".into()),
        });
    }
    view.replace(
        state.catalog.clone(),
        state.backpack.clone(),
        state.loot.clone(),
    );
    pending.0.extend(incoming.read().copied());
}

pub fn quit(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<bevy::app::AppExit>) {
    if keys.just_pressed(KeyCode::KeyQ) {
        exit.write(bevy::app::AppExit::Success);
    }
}
