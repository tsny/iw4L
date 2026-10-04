use std::collections::BTreeMap;

use bevy::{input::InputSystems, prelude::*};

use crate::model::{
    DropAction, DropId, DropRequest, DropResult, InventoryOverlay, InventoryOverlayRequest,
    InventoryView,
};
use crate::{InventoryUiSet, view};

const MODAL_OWNER: &str = "inventory";

#[derive(Resource, Default)]
struct RuntimePending(BTreeMap<sim::ActionRequestId, DropId>);

pub struct InventoryUiPlugin;

impl Plugin for InventoryUiPlugin {
    fn build(&self, app: &mut App) {
        frame::register_ui_contracts(app);
        app.init_resource::<InventoryOverlay>()
            .init_resource::<RuntimePending>()
            .add_message::<DropRequest>()
            .add_message::<DropResult>()
            .add_message::<InventoryOverlayRequest>()
            .add_message::<frame::MatchTornDown>()
            .add_message::<net::ReliableControlEvent>()
            .configure_sets(
                Update,
                (
                    InventoryUiSet::Input,
                    InventoryUiSet::Adapter,
                    InventoryUiSet::Present,
                )
                    .chain(),
            )
            .add_systems(PreStartup, view::load_font)
            .add_systems(
                PreUpdate,
                keyboard_and_modal
                    .after(InputSystems)
                    .in_set(frame::ModalInputSet),
            )
            .add_systems(
                Update,
                (
                    consume_overlay_requests,
                    view::handle_interactions,
                    view::finish_drop,
                )
                    .chain()
                    .in_set(InventoryUiSet::Input),
            )
            .add_systems(
                Update,
                (
                    sync_runtime_view,
                    submit_runtime_drops,
                    resolve_runtime_drops,
                )
                    .chain()
                    .in_set(InventoryUiSet::Adapter),
            )
            .add_systems(
                Update,
                (view::apply_results, teardown, view::rebuild)
                    .chain()
                    .in_set(InventoryUiSet::Present),
            );
    }
}

fn keyboard_and_modal(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    view: Option<Res<InventoryView>>,
    mut overlay: ResMut<InventoryOverlay>,
    mut modal: ResMut<frame::ModalInput>,
    hud_input: Option<Res<frame::HudInputView>>,
) {
    // Tab toggles the overlay in a match. Resetting the key keeps its retail
    // `+scores` bind from also firing. The console keeps Tab for completion.

    let typing = hud_input.is_some_and(|input| input.console_open || input.script_menu_open);
    if view.is_some() && !typing && keys.just_pressed(KeyCode::Tab) {
        let open = !overlay.open;
        overlay.set_open(open);
        keys.reset(KeyCode::Tab);
    }
    if overlay.open && keys.just_pressed(KeyCode::Escape) {
        overlay.set_open(false);
        keys.reset(KeyCode::Escape);
    }
    if overlay.open
        && keys.just_pressed(KeyCode::KeyR)
        && let Some(view) = view
    {
        overlay.status = match overlay.rotate_drag(&view) {
            Ok(()) => "Rotated preview.".into(),
            Err(error) => error,
        };
    }
    if overlay.open {
        modal.claim(MODAL_OWNER);
    } else {
        modal.release(MODAL_OWNER);
    }
}

fn consume_overlay_requests(
    mut requests: MessageReader<InventoryOverlayRequest>,
    mut overlay: ResMut<InventoryOverlay>,
) {
    for request in requests.read() {
        match request {
            InventoryOverlayRequest::Open => overlay.set_open(true),
            InventoryOverlayRequest::Close => overlay.set_open(false),
            InventoryOverlayRequest::Toggle => {
                let open = !overlay.open;
                overlay.set_open(open);
            }
        }
    }
}

fn sync_runtime_view(
    mut commands: Commands,
    presented: Option<Res<net::PresentedSnapshot>>,
    local: Option<Res<net::LocalPresentClient>>,
    current: Option<ResMut<InventoryView>>,
) {
    let (Some(presented), Some(local)) = (presented, local) else {
        return;
    };
    let Some(inventory) = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .and_then(|meta| meta.inventory.as_ref())
    else {
        return;
    };
    let catalog = sim::loot_catalog().clone();
    let player = inventory.backpack().clone();
    if let Some(mut current) = current {
        current.replace(catalog, player, None);
    } else {
        commands.insert_resource(InventoryView {
            catalog,
            player,
            container: None,
        });
    }
}

fn submit_runtime_drops(
    mut requests: MessageReader<DropRequest>,
    local: Option<Res<net::LocalPresentClient>>,
    inbox: Option<ResMut<net::ClientActionInbox>>,
    ids: Option<ResMut<net::ActionRequestIds>>,
    mut pending: ResMut<RuntimePending>,
    mut results: MessageWriter<DropResult>,
) {
    let (Some(local), Some(mut inbox), Some(mut ids)) = (local, inbox, ids) else {
        return;
    };
    for request in requests.read() {
        let request_id = ids.allocate();
        let transaction = match request.action {
            DropAction::Place {
                source,
                destination,
                source_container,
                destination_container: _,
                expected_source_revision,
                expected_destination_revision: _,
                instance,
                target,
            } if source == destination => sim::InventoryTransaction::Move {
                container: source_container,
                expected_revision: expected_source_revision,
                instance,
                target,
            },
            DropAction::Place {
                source_container,
                destination_container,
                expected_source_revision,
                expected_destination_revision,
                instance,
                target,
                ..
            } => sim::InventoryTransaction::Transfer {
                source: source_container,
                destination: destination_container,
                expected_source_revision,
                expected_destination_revision,
                instance,
                amount: sim::InventoryTransferAmount::Whole,
                target,
            },
            DropAction::Merge {
                container,
                expected_revision,
                source_instance,
                destination_instance,
                quantity,
                ..
            } => sim::InventoryTransaction::Merge {
                container,
                expected_revision,
                source_instance,
                destination_instance,
                quantity,
            },
            DropAction::DropToWorld {
                container,
                expected_revision,
                instance,
            } => sim::InventoryTransaction::Drop {
                container,
                expected_revision,
                instance,
            },
        };
        let action = sim::ClientAction::InventoryTransaction {
            request_id,
            transaction,
        };
        match inbox.push(local.0, action) {
            Ok(()) => {
                pending.0.insert(request_id, request.id);
            }
            Err(error) => {
                results.write(DropResult {
                    id: request.id,
                    accepted: false,
                    detail: error.to_string(),
                });
            }
        }
    }
}

fn resolve_runtime_drops(
    mut events: MessageReader<net::ReliableControlEvent>,
    mut pending: ResMut<RuntimePending>,
    mut results: MessageWriter<DropResult>,
) {
    for event in events.read() {
        let (request_id, accepted, detail) = match event.0 {
            sim::SimEvent::InventoryTransactionAccepted { request_id, .. } => {
                (request_id, true, "Inventory updated.".to_owned())
            }
            sim::SimEvent::InventoryTransactionRejected {
                request_id, reason, ..
            } => (request_id, false, reason.to_string()),
            _ => continue,
        };
        let Some(id) = pending.0.remove(&request_id) else {
            continue;
        };
        results.write(DropResult {
            id,
            accepted,
            detail,
        });
    }
}

fn teardown(
    mut torn: MessageReader<frame::MatchTornDown>,
    mut overlay: ResMut<InventoryOverlay>,
    mut pending: ResMut<RuntimePending>,
) {
    if torn.read().count() == 0 {
        return;
    }
    overlay.set_open(false);
    overlay.pending.clear();
    pending.0.clear();
}
