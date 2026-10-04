mod model;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use bevy::prelude::*;
use bevy::window::WindowResolution;
use inventory::Catalog;

use crate::model::LabState;

const BASE_ITEMS: &str = include_str!("../../../content/loot/base/items.json");

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("inventory_lab: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.help {
        println!("usage: cargo run -p inventory_lab -- [--check] [--items PATH]");
        println!("       click an item, press R to rotate, then click a cell or matching stack");
        return Ok(());
    }
    let source = match options.items {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        None => BASE_ITEMS.to_owned(),
    };
    let catalog = Catalog::from_json(&source).map_err(|error| error.to_string())?;
    let state = LabState::seeded(catalog)?;
    if options.check {
        println!("inventory lab: {}", state.check()?);
        return Ok(());
    }

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.025, 0.03, 0.035)))
        .insert_resource(state)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "IW4L inventory lab".into(),
                resolution: WindowResolution::new(1180, 760),
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, ui::setup)
        .add_systems(
            Update,
            (ui::handle_keyboard, ui::handle_interactions, ui::rebuild_ui).chain(),
        )
        .add_systems(Update, ui::update_button_visuals)
        .run();
    Ok(())
}

#[derive(Default)]
struct Options {
    check: bool,
    help: bool,
    items: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--check" => out.check = true,
                "-h" | "--help" => out.help = true,
                "--items" => {
                    let value = args.next().ok_or("--items requires a path")?;
                    if out.items.replace(PathBuf::from(value)).is_some() {
                        return Err("--items may be supplied only once".into());
                    }
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        Ok(out)
    }
}
