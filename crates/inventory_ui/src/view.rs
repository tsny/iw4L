use bevy::prelude::*;
use inventory::{ItemDefId, ItemInstanceId, MAX_CONDITION, UseEffect};
use ui::UiLayer;

use crate::model::{
    DropRequest, DropResult, HoverTarget, InventoryOverlay, InventoryPane, InventoryView,
    evaluate_drop,
};

const FONT_BYTES: &[u8] = include_bytes!("../../ui/assets/Oxanium-Regular.ttf");
const CELL: f32 = 52.0;
const INSET: f32 = 2.0;

#[derive(Resource)]
pub(crate) struct InventoryFont(pub Handle<Font>);

#[derive(Component)]
pub(crate) struct InventoryRoot;

#[derive(Component, Clone, Copy)]
pub(crate) struct GridCell {
    pane: InventoryPane,
    x: u8,
    y: u8,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct ItemTile {
    pane: InventoryPane,
    instance: ItemInstanceId,
}

type InteractionQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        Option<&'static GridCell>,
        Option<&'static ItemTile>,
    ),
    Changed<Interaction>,
>;

pub(crate) fn load_font(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    commands.insert_resource(InventoryFont(
        fonts.add(Font::from_bytes(FONT_BYTES.to_vec())),
    ));
}

pub(crate) fn handle_interactions(
    view: Option<Res<InventoryView>>,
    mut overlay: ResMut<InventoryOverlay>,
    interactions: InteractionQuery,
) {
    if !overlay.open {
        return;
    }
    let Some(view) = view else { return };
    for (interaction, cell, item) in &interactions {
        let target = cell
            .map(|cell| HoverTarget::Cell {
                pane: cell.pane,
                x: cell.x,
                y: cell.y,
            })
            .or_else(|| {
                item.map(|item| HoverTarget::Item {
                    pane: item.pane,
                    instance: item.instance,
                })
            });
        match interaction {
            Interaction::Hovered | Interaction::Pressed => overlay.hover = target,
            Interaction::None if overlay.hover == target => overlay.hover = None,
            Interaction::None => {}
        }
        let (Interaction::Pressed, Some(item)) = (interaction, item) else {
            continue;
        };
        overlay.status = match overlay.begin_drag(&view, item.pane, item.instance) {
            Ok(()) => "Drag to a highlighted cell; R rotates the preview.".into(),
            Err(error) => error,
        };
    }
}

pub(crate) fn finish_drop(
    mouse: Res<ButtonInput<MouseButton>>,
    view: Option<Res<InventoryView>>,
    mut overlay: ResMut<InventoryOverlay>,
    mut requests: MessageWriter<DropRequest>,
) {
    if !overlay.open || !mouse.just_released(MouseButton::Left) {
        return;
    }
    let Some(view) = view else {
        return;
    };
    match overlay.complete_drop(&view) {
        Ok(request) => {
            overlay.status = "Pending authority…".into();
            requests.write(request);
        }
        Err(error) => overlay.status = error,
    }
}

pub(crate) fn apply_results(
    mut results: MessageReader<DropResult>,
    mut overlay: ResMut<InventoryOverlay>,
) {
    for result in results.read() {
        overlay.resolve(result);
    }
}

pub(crate) fn rebuild(
    mut commands: Commands,
    overlay: Res<InventoryOverlay>,
    view: Option<Res<InventoryView>>,
    font: Option<Res<InventoryFont>>,
    old: Query<Entity, With<InventoryRoot>>,
) {
    if !overlay.is_changed() && !view.as_ref().is_some_and(|view| view.is_changed()) {
        return;
    }
    for entity in &old {
        commands.entity(entity).try_despawn();
    }
    if !overlay.open {
        return;
    }
    let (Some(view), Some(font)) = (view, font) else {
        return;
    };
    commands
        .spawn((
            InventoryRoot,
            UiLayer::Overlay,
            GlobalZIndex(10_000),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(28)),
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.02, 0.022, 0.94)),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("FIELD INVENTORY"),
                text_font(&font.0, 28.0),
                TextColor(Color::srgb(0.88, 0.82, 0.56)),
            ));
            root.spawn((
                Text::new("Drag item · R rotate · Esc close"),
                text_font(&font.0, 14.0),
                TextColor(Color::srgb(0.60, 0.65, 0.64)),
            ));
            root.spawn((
                Node {
                    padding: UiRect::all(px(10)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.07, 0.085, 0.085)),
            ))
            .with_child((
                Text::new(format!(
                    "{}\n{}",
                    overlay.status,
                    detail_text(&view, &overlay)
                )),
                text_font(&font.0, 14.0),
                TextColor(Color::srgb(0.84, 0.86, 0.82)),
            ));
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: px(28),
                ..default()
            })
            .with_children(|row| {
                spawn_panel(row, &view, &overlay, InventoryPane::Player, &font.0);
                spawn_panel(row, &view, &overlay, InventoryPane::Container, &font.0);
            });
        });
}

fn spawn_panel(
    parent: &mut ChildSpawnerCommands,
    view: &InventoryView,
    overlay: &InventoryOverlay,
    pane: InventoryPane,
    font: &Handle<Font>,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(7),
            ..default()
        })
        .with_children(|panel| {
            let Some(grid) = view.grid(pane) else {
                panel.spawn((
                    Text::new(format!("{}\nNo container open", pane.label())),
                    text_font(font, 17.0),
                    TextColor(Color::srgb(0.55, 0.58, 0.56)),
                ));
                return;
            };
            let weight = grid.total_weight_g(&view.catalog).unwrap_or(0);
            panel.spawn((
                Text::new(format!(
                    "{}  {}×{}  ·  {} items  ·  {:.2} kg  ·  rev {}",
                    pane.label(),
                    grid.width(),
                    grid.height(),
                    grid.items().len(),
                    weight as f32 / 1000.0,
                    grid.revision()
                )),
                text_font(font, 17.0),
                TextColor(Color::srgb(0.82, 0.84, 0.78)),
            ));
            panel
                .spawn((
                    Node {
                        width: px(f32::from(grid.width()) * CELL),
                        height: px(f32::from(grid.height()) * CELL),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.035, 0.042, 0.045)),
                ))
                .with_children(|canvas| {
                    for y in 0..grid.height() {
                        for x in 0..grid.width() {
                            canvas.spawn((
                                Button,
                                GridCell { pane, x, y },
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(f32::from(x) * CELL + INSET),
                                    top: px(f32::from(y) * CELL + INSET),
                                    width: px(CELL - INSET * 2.0),
                                    height: px(CELL - INSET * 2.0),
                                    border: UiRect::all(px(1)),
                                    ..default()
                                },
                                BackgroundColor(cell_color(x, y)),
                                BorderColor::all(Color::srgb(0.15, 0.18, 0.18)),
                            ));
                        }
                    }
                    for item in grid.items() {
                        let (Some(def), Some(place)) = (
                            view.catalog.definition(item.definition),
                            grid.placement(item.id),
                        ) else {
                            continue;
                        };
                        let size = def.footprint.oriented(place.rotated);
                        let dragging = overlay.drag.is_some_and(|drag| drag.instance == item.id);
                        let pending = overlay.pending.contains_key(&item.id);
                        let caption = format!(
                            "{}{}{}",
                            def.name,
                            if item.quantity > 1 {
                                format!("\n×{}", item.quantity)
                            } else {
                                String::new()
                            },
                            if pending { "\nPENDING" } else { "" }
                        );
                        canvas
                            .spawn((
                                Button,
                                ItemTile {
                                    pane,
                                    instance: item.id,
                                },
                                ImageNode::solid_color(item_color(
                                    item.definition,
                                    dragging,
                                    pending,
                                )),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(f32::from(place.x) * CELL + INSET + 1.0),
                                    top: px(f32::from(place.y) * CELL + INSET + 1.0),
                                    width: px(f32::from(size.width) * CELL - INSET * 2.0 - 2.0),
                                    height: px(f32::from(size.height) * CELL - INSET * 2.0 - 2.0),
                                    border: UiRect::all(px(if dragging || pending {
                                        3
                                    } else {
                                        1
                                    })),
                                    padding: UiRect::all(px(5)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BorderColor::all(if pending {
                                    Color::srgb(0.85, 0.55, 0.20)
                                } else if dragging {
                                    Color::srgb(0.98, 0.80, 0.26)
                                } else {
                                    Color::srgb(0.26, 0.31, 0.30)
                                }),
                                ZIndex(2),
                            ))
                            .with_child((
                                Text::new(caption),
                                text_font(font, 12.0),
                                TextColor(Color::srgb(0.96, 0.97, 0.91)),
                                TextLayout::justify(Justify::Center),
                            ));
                    }
                    spawn_ghost(canvas, view, overlay, pane);
                });
        });
}

fn spawn_ghost(
    canvas: &mut ChildSpawnerCommands,
    view: &InventoryView,
    overlay: &InventoryOverlay,
    pane: InventoryPane,
) {
    let (
        Some(drag),
        Some(HoverTarget::Cell {
            pane: target_pane,
            x,
            y,
        }),
    ) = (overlay.drag, overlay.hover)
    else {
        return;
    };
    if pane != target_pane {
        return;
    }
    let Some(item) = view
        .grid(drag.pane)
        .and_then(|grid| grid.item(drag.instance))
    else {
        return;
    };
    let Some(def) = view.catalog.definition(item.definition) else {
        return;
    };
    let size = def.footprint.oriented(drag.rotated);
    let valid = evaluate_drop(view, drag, HoverTarget::Cell { pane, x, y }).is_ok();
    canvas.spawn((
        ImageNode::solid_color(if valid {
            Color::srgba(0.25, 0.75, 0.32, 0.58)
        } else {
            Color::srgba(0.85, 0.20, 0.16, 0.58)
        }),
        bevy::picking::Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: px(f32::from(x) * CELL + INSET),
            top: px(f32::from(y) * CELL + INSET),
            width: px(f32::from(size.width) * CELL - INSET * 2.0),
            height: px(f32::from(size.height) * CELL - INSET * 2.0),
            ..default()
        },
        ZIndex(4),
    ));
}

fn detail_text(view: &InventoryView, overlay: &InventoryOverlay) -> String {
    let target = overlay
        .hover
        .and_then(|hover| match hover {
            HoverTarget::Item { pane, instance } => Some((pane, instance)),
            HoverTarget::Cell { .. } => None,
        })
        .or_else(|| overlay.drag.map(|drag| (drag.pane, drag.instance)));
    let Some((pane, instance)) = target else {
        return "Hover an item for details".into();
    };
    let Some(item) = view.grid(pane).and_then(|grid| grid.item(instance)) else {
        return "Item unavailable".into();
    };
    let Some(def) = view.catalog.definition(item.definition) else {
        return "Definition unavailable".into();
    };
    let effect = match def.use_effect {
        None => "no effect".into(),
        Some(UseEffect::Heal { amount }) => format!("heal {amount}"),
        Some(UseEffect::RestoreHeldAmmo { amount }) => format!("restore {amount} ammo"),
    };
    format!(
        "{} · {} · {} g each · condition {}/{} · {}",
        def.name, def.key, def.weight_g, item.condition, MAX_CONDITION, effect
    )
}

fn text_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}
fn cell_color(x: u8, y: u8) -> Color {
    if (x + y).is_multiple_of(2) {
        Color::srgb(0.075, 0.085, 0.085)
    } else {
        Color::srgb(0.065, 0.075, 0.075)
    }
}
fn item_color(definition: ItemDefId, dragging: bool, pending: bool) -> Color {
    if pending {
        return Color::srgb(0.38, 0.27, 0.12);
    }
    if dragging {
        return Color::srgb(0.45, 0.37, 0.14);
    }
    match definition.0 % 4 {
        0 => Color::srgb(0.26, 0.32, 0.24),
        1 => Color::srgb(0.34, 0.23, 0.19),
        2 => Color::srgb(0.18, 0.29, 0.31),
        _ => Color::srgb(0.31, 0.27, 0.18),
    }
}
