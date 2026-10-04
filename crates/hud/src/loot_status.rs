use std::collections::HashMap;

use asset_game::MenuCatalog;
use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, tessellate_fonts};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::images::HudImages;
use crate::presentation_scale::{HorizontalAlign, VerticalAlign};

#[derive(Component)]
pub(crate) struct LootStatusRaster;

#[derive(Default)]
pub(crate) struct ToastMemory {
    notice: Option<(u32, u32)>,
    shown_at: f64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    surface: Res<crate::surface::Hud2dSurface>,
    catalog: Option<Res<MenuCatalog>>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    input: Res<frame::HudInputView>,
    view: Option<Res<frame::ViewSubject>>,
    time: Res<Time>,
    mut pass: ResMut<HudTessPass>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut memory: Local<ToastMemory>,
) {
    pass.loot_status = TessJob::Hide;
    if !surface.is_ready()
        || input.script_menu_open
        || input.console_open
        || view.as_ref().is_some_and(|view| view.in_killcam())
    {
        return;
    }
    let Some(meta) = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .filter(|meta| meta.lifecycle == sim::ClientLifecycle::Alive)
    else {
        return;
    };
    let Some(inventory) = meta.inventory.as_ref() else {
        return;
    };
    let Ok(summary) = inventory.summary() else {
        return;
    };
    let Some(catalog) = catalog.as_ref() else {
        return;
    };
    let Some(item) = catalog
        .get("hud_fullscreen")
        .and_then(|menu| menu.items.iter().find(|item| item.owner_draw == 113))
    else {
        return;
    };
    let font_name = hud_iw4::ui_get_font_handle(
        item.font_enum,
        surface.scale_virtual_to_real()[1],
        item.text_scale,
    );
    let Some(font) = catalog.font(font_name) else {
        return;
    };
    let scale = hud_iw4::normalized_text_scale(font.pixel_height, item.text_scale * 0.78);
    let material = asset_core::AssetRef::bare_name(&font.material).to_owned();
    let mut commands = Vec::new();
    push_text(
        &mut commands,
        &surface,
        font_name,
        &material,
        format!(
            "BACKPACK  {:.2} KG  ·  {}/{} CELLS",
            summary.weight_g as f64 / 1000.0,
            summary.used_cells,
            summary.total_cells
        ),
        scale,
        [0.80, 0.82, 0.68, 0.90],
        18.0,
        -22.0,
        HorizontalAlign::Left as i32,
        VerticalAlign::Bottom as i32,
    );

    if let Some(notice) = inventory.latest_notice() {
        let identity = (notice.request_id, notice.revision);
        if memory.notice != Some(identity) {
            memory.notice = Some(identity);
            memory.shown_at = time.elapsed_secs_f64();
        }
        let age = time.elapsed_secs_f64() - memory.shown_at;
        if age < 2.6
            && let Some(definition) = sim::loot_catalog().definition(notice.definition)
        {
            let alpha = (1.0 - ((age - 1.8) / 0.8).max(0.0)).clamp(0.0, 1.0) as f32;
            let text = format!(
                "{}  +{}",
                definition.name.to_ascii_uppercase(),
                notice.quantity
            );
            let toast_scale = hud_iw4::normalized_text_scale(font.pixel_height, item.text_scale);
            let width = crate::chrome::ui_text_width(font, &text, item.text_scale);
            push_text(
                &mut commands,
                &surface,
                font_name,
                &material,
                text,
                toast_scale,
                [0.90, 0.80, 0.42, alpha],
                -width * 0.5,
                116.0,
                HorizontalAlign::Center as i32,
                VerticalAlign::Top as i32,
            );
        }
    }

    let list = Draw2dList { cmds: commands };
    let fonts = HashMap::from([(font_name.to_owned(), font)]);
    let (quads, _) = tessellate_fonts(&list, &fonts);
    if !quads.is_empty()
        && hud_images
            .get(crate::images::HUD_CHROME_NAMESPACE, &material, &mut images)
            .is_some()
    {
        pass.loot_status = TessJob::Quads(quads);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_text(
    commands: &mut Vec<Draw2dCmd>,
    surface: &crate::surface::Hud2dSurface,
    font: &str,
    material: &str,
    text: String,
    scale: f32,
    color: [f32; 4],
    x: f32,
    y: f32,
    horizontal: i32,
    vertical: i32,
) {
    let rect = surface.apply_rect(x, y, scale, scale, horizontal, vertical);
    commands.push(Draw2dCmd {
        material_namespace: crate::images::HUD_CHROME_NAMESPACE,
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        s0: 0.0,
        t0: 0.0,
        s1: 1.0,
        t1: 1.0,
        color,
        material: material.to_owned(),
        op: Draw2dOp::TextRun {
            font: font.to_owned(),
            scale,
            text,
            loc_key: String::new(),
            style: 0,
            fx: None,
            glow: None,
        },
        provenance: Draw2dProvenance::CgDraw {
            site: "loot_status",
        },
        layer: 1,
    });
}
