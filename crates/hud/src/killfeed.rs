use std::collections::{HashMap, HashSet, VecDeque};

use asset_game::MenuCatalog;
use assets::PreparedLocalizedStrings;
use bevy::prelude::*;
use hud_iw4::{
    GAME_MSG_WIN0_HORZ_ALIGN, GAME_MSG_WIN0_LINE_COUNT, GAME_MSG_WIN0_MSG_TIME_MS,
    GAME_MSG_WIN0_TEXT_SCALE, GAME_MSG_WIN0_TEXT_STYLE, GAME_MSG_WIN0_VERT_ALIGN, GAME_MSG_WIN0_X,
    game_msg_win0_line_y, gamenotify_line, normalized_text_scale,
};
use net::LocalPresentClient;

use crate::chrome::text_width;
use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, tessellate_fonts};
use crate::font_overlay::HUD_SMALL_FONT;
use crate::gaps::{GapCause, HudGap, HudPresentationGaps};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::images::HudImages;
use crate::scorebar::milliseconds;

#[derive(Clone, Debug)]
enum KillfeedLine {
    Notify {
        start_ms: i32,
        text: String,
        name_empty: bool,
    },
}

impl KillfeedLine {
    fn start_ms(&self) -> i32 {
        match self {
            Self::Notify { start_ms, .. } => *start_ms,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct KillfeedWindow {
    lines: VecDeque<KillfeedLine>,
    bold: VecDeque<(i32, String)>,
    seen: HashSet<u32>,
}

const BOLD_LINE_COUNT: usize = 3;
const BOLD_MSG_TIME_MS: i32 = 3000;
const BOLD_TEXT_SCALE: f32 = 0.5;
const BOLD_Y: f32 = 100.0;

#[derive(Component)]
pub(crate) struct KillfeedRaster;

pub(crate) fn spawn_killfeed(root: &mut ChildSpawnerCommands) {
    crate::font_overlay::spawn_overlay(root, KillfeedRaster);
}

fn hide(pass: &mut HudTessPass) {
    pass.killfeed = TessJob::Hide;
}

fn text_cmd(
    x: f32,
    y: f32,
    cmd_w: f32,
    cmd_h: f32,
    material: String,
    text: String,
    color: [f32; 4],
    site: &'static str,
) -> Draw2dCmd {
    Draw2dCmd {
        material_namespace: crate::images::HUD_CHROME_NAMESPACE,
        x: (x + 0.5).floor(),
        y: (y + 0.5).floor(),
        w: cmd_w,
        h: cmd_h,
        s0: 0.0,
        t0: 0.0,
        s1: 1.0,
        t1: 1.0,
        color,
        material,
        op: Draw2dOp::TextRun {
            font: HUD_SMALL_FONT.to_owned(),
            scale: cmd_w,
            text,
            loc_key: String::new(),

            style: GAME_MSG_WIN0_TEXT_STYLE,
            fx: None,
            glow: None,
        },
        provenance: Draw2dProvenance::CgDraw { site },
        layer: 1,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_killfeed(
    surface: Res<crate::surface::Hud2dSurface>,
    presented: Res<net::PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    catalog: Option<Res<MenuCatalog>>,
    strings: Option<Res<PreparedLocalizedStrings>>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut window: ResMut<KillfeedWindow>,
    mut pass: ResMut<HudTessPass>,
    mut notifies: MessageReader<net::SvcGameNotify>,
    view: Option<Res<frame::ViewSubject>>,
) {
    if !surface.is_ready() {
        hide(&mut pass);
        return;
    }
    if presented.player(local.0).is_none() {
        window.lines.clear();
        window.bold.clear();
        window.seen.clear();
        gaps.clear(HudGap::Obituary);
        hide(&mut pass);
        return;
    }

    let now = milliseconds() as i32;
    for cmd in notifies.read() {
        if !window.seen.insert(cmd.id) {
            continue;
        }
        let Some(template) = strings
            .as_ref()
            .and_then(|s| crate::hudelem::resolve_hud_text(s, &cmd.key))
        else {
            gaps.raise(GapCause::LocalizedRowMissing {
                key: cmd.key.clone(),
            });
            continue;
        };
        let args: Vec<String> = cmd
            .name
            .split(sim::HUD_PRINT_ARG_SEPARATOR)
            .map(|arg| {
                strings
                    .as_ref()
                    .and_then(|s| crate::hudelem::resolve_hud_text(s, arg))
                    .unwrap_or_else(|| arg.to_owned())
            })
            .collect();
        let text = if template.contains("&&2") {
            args.iter().enumerate().fold(template, |line, (i, arg)| {
                line.replace(&format!("&&{}", i + 1), arg)
            })
        } else {
            gamenotify_line(&template, &args[0])
        };
        if cmd.tag == net::SVC_PRINT_BOLD {
            window.bold.push_back((now, text));
            while window.bold.len() > BOLD_LINE_COUNT {
                window.bold.pop_front();
            }
            continue;
        }
        window.lines.push_back(KillfeedLine::Notify {
            start_ms: now,
            text,
            name_empty: cmd.tag == net::SVC_DISCONNECT_NOTIFY && cmd.name.is_empty(),
        });
        while window.lines.len() > GAME_MSG_WIN0_LINE_COUNT {
            window.lines.pop_front();
        }
    }
    window
        .lines
        .retain(|line| now.saturating_sub(line.start_ms()) < GAME_MSG_WIN0_MSG_TIME_MS);
    window
        .bold
        .retain(|(start, _)| now.saturating_sub(*start) < BOLD_MSG_TIME_MS);
    if window.lines.is_empty() && window.bold.is_empty() {
        gaps.clear(HudGap::Obituary);
        hide(&mut pass);
        return;
    }
    if view.is_some_and(|v| v.in_killcam()) {
        hide(&mut pass);
        return;
    }

    let names_ok = match window.lines.back() {
        None => true,
        Some(KillfeedLine::Notify { text, .. }) => !text.is_empty(),
    };
    match window.lines.back() {
        None => {}
        Some(KillfeedLine::Notify { name_empty, .. }) => {
            if *name_empty {
                gaps.raise(GapCause::GameNotifyNoClientInfo);
            } else if names_ok {
                gaps.clear(HudGap::Obituary);
            }
        }
    }

    let font = catalog.as_ref().and_then(|c| c.font(HUD_SMALL_FONT));
    let (nscale, font_material) = match font {
        Some(def) => (
            normalized_text_scale(def.pixel_height, GAME_MSG_WIN0_TEXT_SCALE),
            asset_core::AssetRef::bare_name(&def.material).to_owned(),
        ),
        None => (0.0, String::new()),
    };
    if names_ok && font.is_none() {
        gaps.raise(GapCause::ObituaryNoClientInfo);
    }

    let mut cmds = Vec::new();
    let mut fonts = HashMap::new();
    let font_tex_ok = if let Some(def) = font {
        fonts.insert(HUD_SMALL_FONT.to_owned(), def);
        hud_images
            .get(
                crate::images::HUD_CHROME_NAMESPACE,
                &font_material,
                &mut images,
            )
            .is_some()
    } else {
        false
    };
    if names_ok && font.is_some() && !font_tex_ok {
        let image = hud_images
            .zone_image_name(&font_material)
            .map(str::to_owned);
        gaps.raise(GapCause::FontAtlasMissing {
            material: font_material.clone(),
            image,
        });
    }

    for (i, line) in window.lines.iter().rev().enumerate() {
        let first_cmd = cmds.len();
        let age = now.saturating_sub(line.start_ms());

        let alpha = (age as f32 / 250.0).clamp(0.0, 1.0)
            * ((GAME_MSG_WIN0_MSG_TIME_MS - age) as f32 / 500.0).clamp(0.0, 1.0);
        let y_virtual = game_msg_win0_line_y(i);
        match line {
            KillfeedLine::Notify { text, .. } => {
                if font.is_some() && font_tex_ok {
                    let applied = surface.apply_rect(
                        GAME_MSG_WIN0_X,
                        y_virtual,
                        nscale,
                        nscale,
                        GAME_MSG_WIN0_HORZ_ALIGN,
                        GAME_MSG_WIN0_VERT_ALIGN,
                    );
                    cmds.push(text_cmd(
                        applied.x,
                        applied.y,
                        applied.w,
                        applied.h,
                        font_material.clone(),
                        text.clone(),
                        [1.0; 4],
                        "killfeed_game_msg",
                    ));
                }
            }
        }
        for cmd in &mut cmds[first_cmd..] {
            cmd.color[3] *= alpha;
        }
    }
    if let Some(def) = font.filter(|_| font_tex_ok) {
        let scale = normalized_text_scale(def.pixel_height, BOLD_TEXT_SCALE);
        for (i, (start, text)) in window.bold.iter().enumerate() {
            let age = now.saturating_sub(*start);
            let alpha = ((BOLD_MSG_TIME_MS - age) as f32 / 500.0).clamp(0.0, 1.0);
            let width = text_width(def, text) as f32 * scale;
            let line_h = def.pixel_height as f32 * scale;
            let applied = surface.apply_rect(
                -width / 2.0,
                BOLD_Y + line_h * i as f32,
                scale,
                scale,
                hud_iw4::ALIGN_CENTER,
                hud_iw4::ALIGN_VIEWABLE,
            );
            cmds.push(text_cmd(
                applied.x,
                applied.y,
                applied.w,
                applied.h,
                font_material.clone(),
                text.clone(),
                [1.0, 1.0, 1.0, alpha],
                "killfeed_bold_msg",
            ));
        }
    }

    let list = Draw2dList { cmds };
    let (quads, _) = tessellate_fonts(&list, &fonts);
    if quads.is_empty() {
        hide(&mut pass);
        return;
    }
    pass.killfeed = TessJob::Quads(quads);
}
