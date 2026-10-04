use bevy::app::AppExit;
use bevy::prelude::*;

use inventory::{ItemDefId, ItemInstanceId, UseEffect};

use crate::model::{LabState, Pane};

const FONT_BYTES: &[u8] = include_bytes!("../../ui/assets/Oxanium-Regular.ttf");
const CELL: f32 = 52.0;
const CELL_INSET: f32 = 2.0;

#[derive(Resource)]
pub(crate) struct LabFont(Handle<Font>);

#[derive(Component)]
pub(crate) struct LabRoot;

#[derive(Component, Clone, Copy)]
pub(crate) struct CellButton {
    pane: Pane,
    x: u8,
    y: u8,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct ItemButton {
    pane: Pane,
    instance: ItemInstanceId,
    definition: ItemDefId,
    selected: bool,
}

type LabInteractionQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        Option<&'static CellButton>,
        Option<&'static ItemButton>,
    ),
    Changed<Interaction>,
>;

type LabButtonVisualQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        Option<&'static CellButton>,
        Option<&'static ItemButton>,
        Option<&'static mut BackgroundColor>,
        Option<&'static mut ImageNode>,
    ),
    Changed<Interaction>,
>;

pub fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    commands.spawn(Camera2d);
    commands.insert_resource(LabFont(fonts.add(Font::from_bytes(FONT_BYTES.to_vec()))));
}

pub fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LabState>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        state.rotate_selection();
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.clear_selection();
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        exit.write(AppExit::Success);
    }
}

pub fn handle_interactions(mut state: ResMut<LabState>, interactions: LabInteractionQuery) {
    for (interaction, cell, item) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(item) = item {
            state.activate_item(item.pane, item.instance);
        } else if let Some(cell) = cell {
            state.activate_cell(cell.pane, cell.x, cell.y);
        }
    }
}

pub fn rebuild_ui(
    mut commands: Commands,
    state: Res<LabState>,
    font: Option<Res<LabFont>>,
    old: Query<Entity, With<LabRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    let Some(font) = font else {
        return;
    };
    for entity in &old {
        commands.entity(entity).try_despawn();
    }

    commands
        .spawn((
            LabRoot,
            Node {
                width: percent(100),
                min_height: percent(100),
                padding: UiRect::all(px(24)),
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                ..default()
            },
            BackgroundColor(Color::srgb(0.025, 0.03, 0.035)),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("IW4L INVENTORY LAB"),
                text_font(&font.0, 30.0),
                TextColor(Color::srgb(0.88, 0.82, 0.56)),
            ));
            root.spawn((
                Text::new(
                    "Asset-free authoring preview · click item → R rotates → click cell/stack · Esc clears · Q quits",
                ),
                text_font(&font.0, 15.0),
                TextColor(Color::srgb(0.62, 0.66, 0.67)),
            ));
            root.spawn((
                Node {
                    width: percent(100),
                    padding: UiRect::all(px(10)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.08, 0.095, 0.10)),
            ))
            .with_child((
                Text::new(format!("{}\n{}", state.status, state.selected_detail())),
                text_font(&font.0, 14.0),
                TextColor(Color::srgb(0.83, 0.85, 0.82)),
            ));
            root.spawn(Node {
                width: percent(100),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: px(28),
                ..default()
            })
            .with_children(|row| {
                spawn_grid_panel(row, &state, Pane::Backpack, &font.0);
                spawn_grid_panel(row, &state, Pane::Loot, &font.0);
            });
            root.spawn((
                Text::new(catalog_line(&state)),
                text_font(&font.0, 13.0),
                TextColor(Color::srgb(0.55, 0.60, 0.60)),
            ));
        });
}

fn spawn_grid_panel(
    parent: &mut ChildSpawnerCommands,
    state: &LabState,
    pane: Pane,
    font: &Handle<Font>,
) {
    let container = state.container(pane);
    let weight = container.total_weight_g(&state.catalog).unwrap_or(0);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(7),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn((
                Text::new(format!(
                    "{}  {}×{}  ·  {} items  ·  {:.2} kg  ·  rev {}",
                    pane.label(),
                    container.width(),
                    container.height(),
                    container.items().len(),
                    weight as f32 / 1000.0,
                    container.revision(),
                )),
                text_font(font, 17.0),
                TextColor(Color::srgb(0.82, 0.84, 0.78)),
            ));
            panel
                .spawn((
                    Node {
                        width: px(f32::from(container.width()) * CELL),
                        height: px(f32::from(container.height()) * CELL),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.035, 0.042, 0.045)),
                ))
                .with_children(|grid| {
                    for y in 0..container.height() {
                        for x in 0..container.width() {
                            grid.spawn((
                                Button,
                                CellButton { pane, x, y },
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(f32::from(x) * CELL + CELL_INSET),
                                    top: px(f32::from(y) * CELL + CELL_INSET),
                                    width: px(CELL - CELL_INSET * 2.0),
                                    height: px(CELL - CELL_INSET * 2.0),
                                    border: UiRect::all(px(1)),
                                    ..default()
                                },
                                BackgroundColor(cell_color(x, y)),
                                BorderColor::all(Color::srgb(0.15, 0.18, 0.18)),
                            ));
                        }
                    }
                    for item in container.items() {
                        let Some(definition) = state.catalog.definition(item.definition) else {
                            continue;
                        };
                        let Some(placement) = container.placement(item.id) else {
                            continue;
                        };
                        let size = definition.footprint.oriented(placement.rotated);
                        let selected = state.selected.is_some_and(|selected| {
                            selected.pane == pane && selected.instance == item.id
                        });
                        let shown_size = if selected {
                            definition
                                .footprint
                                .oriented(state.selected.is_some_and(|selected| selected.rotated))
                        } else {
                            size
                        };
                        let caption = if item.quantity > 1 {
                            format!("{}\n×{}", definition.name, item.quantity)
                        } else {
                            definition.name.clone()
                        };
                        grid.spawn((
                            Button,
                            ItemButton {
                                pane,
                                instance: item.id,
                                definition: item.definition,
                                selected,
                            },
                            ImageNode::solid_color(item_color(item.definition, selected)),
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(f32::from(placement.x) * CELL + CELL_INSET + 1.0),
                                top: px(f32::from(placement.y) * CELL + CELL_INSET + 1.0),
                                width: px(f32::from(shown_size.width) * CELL
                                    - CELL_INSET * 2.0
                                    - 2.0),
                                height: px(f32::from(shown_size.height) * CELL
                                    - CELL_INSET * 2.0
                                    - 2.0),
                                border: UiRect::all(px(if selected { 3 } else { 1 })),
                                border_radius: BorderRadius::all(px(4)),
                                padding: UiRect::all(px(5)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BorderColor::all(if selected {
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
                });
        });
}

pub fn update_button_visuals(mut buttons: LabButtonVisualQuery) {
    for (interaction, cell, item, background, image) in &mut buttons {
        if let (Some(cell), Some(mut background)) = (cell, background) {
            background.0 = match interaction {
                Interaction::Pressed => Color::srgb(0.23, 0.27, 0.24),
                Interaction::Hovered => Color::srgb(0.16, 0.19, 0.18),
                Interaction::None => cell_color(cell.x, cell.y),
            };
        }
        if let (Some(item), Some(mut image)) = (item, image) {
            image.color = match interaction {
                Interaction::Pressed => Color::srgb(0.78, 0.70, 0.38),
                Interaction::Hovered => brighten(item_color(item.definition, item.selected)),
                Interaction::None => item_color(item.definition, item.selected),
            };
        }
    }
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

fn item_color(definition: ItemDefId, selected: bool) -> Color {
    if selected {
        return Color::srgb(0.45, 0.37, 0.14);
    }
    match definition.0 % 4 {
        0 => Color::srgb(0.26, 0.32, 0.24),
        1 => Color::srgb(0.34, 0.23, 0.19),
        2 => Color::srgb(0.18, 0.29, 0.31),
        _ => Color::srgb(0.31, 0.27, 0.18),
    }
}

fn brighten(color: Color) -> Color {
    let linear = color.to_linear();
    Color::LinearRgba(LinearRgba::new(
        (linear.red + 0.12).min(1.0),
        (linear.green + 0.12).min(1.0),
        (linear.blue + 0.12).min(1.0),
        linear.alpha,
    ))
}

fn catalog_line(state: &LabState) -> String {
    let rows = state
        .catalog
        .definitions()
        .iter()
        .map(|definition| {
            let effect = match definition.use_effect {
                None => "inert".to_owned(),
                Some(UseEffect::Heal { amount }) => format!("heal {amount}"),
                Some(UseEffect::RestoreHeldAmmo { amount }) => format!("ammo {amount}"),
            };
            format!(
                "{} {}×{} stack {} / {} g / {}",
                definition.name,
                definition.footprint.width,
                definition.footprint.height,
                definition.max_stack,
                definition.weight_g,
                effect
            )
        })
        .collect::<Vec<_>>()
        .join("   ·   ");
    format!(
        "Catalog schema {} · digest {:016x}\n{}",
        state.catalog.schema(),
        state.catalog.digest(),
        rows
    )
}
