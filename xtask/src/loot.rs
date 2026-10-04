use std::path::{Path, PathBuf};

use inventory::{
    Catalog, ContainerId, GridContainer, ItemInstance, ItemInstanceId, MAX_CONDITION, UseEffect,
};

use crate::shell::Res;

const DEFAULT_ITEMS: &str = "content/loot/base/items.json";

pub fn run_cli(root: &Path, args: &[String]) -> Res<()> {
    let Some(verb) = args.first().map(String::as_str) else {
        return Err("usage: cargo xtask loot validate [items.json]".into());
    };
    if verb != "validate" || args.len() > 2 {
        return Err("usage: cargo xtask loot validate [items.json]".into());
    }
    let path = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ITEMS));
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    validate(&path)
}

fn validate(path: &Path) -> Res<()> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let catalog =
        Catalog::from_json(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    catalog
        .check_invariants()
        .map_err(|error| format!("{}: {error}", path.display()))?;

    println!("loot catalog: {}", path.display());
    println!(
        "schema={} items={} digest={:016x} canonical_bytes={}",
        catalog.schema(),
        catalog.definitions().len(),
        catalog.digest(),
        catalog.canonical_bytes().len()
    );
    for definition in catalog.definitions() {
        let effect = match definition.use_effect {
            None => "none".to_owned(),
            Some(UseEffect::Heal { amount }) => format!("heal:{amount}"),
            Some(UseEffect::RestoreHeldAmmo { amount }) => {
                format!("restore_held_ammo:{amount}")
            }
        };
        println!(
            "item id={} key={} size={}x{} rotate={} stack={} weight_g={} effect={} icon={} model={}",
            definition.id.0,
            definition.key,
            definition.footprint.width,
            definition.footprint.height,
            definition.rotatable,
            definition.max_stack,
            definition.weight_g,
            effect,
            definition
                .icon
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "placeholder".into()),
            definition
                .world_model
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "placeholder".into()),
        );
    }

    let mut preview = GridContainer::new(ContainerId(1), 8, 6)
        .map_err(|error| format!("preview container: {error}"))?;
    for (index, definition) in catalog.definitions().iter().enumerate() {
        let instance = ItemInstance {
            id: ItemInstanceId(index as u64 + 1),
            definition: definition.id,
            quantity: 1,
            condition: MAX_CONDITION,
        };
        let target = preview
            .insert_first_fit(instance, &catalog)
            .map_err(|error| format!("preview placement for `{}`: {error}", definition.key))?;
        println!(
            "layout key={} at={},{} rotated={}",
            definition.key, target.x, target.y, target.rotated
        );
    }
    preview
        .check_invariants(&catalog)
        .map_err(|error| format!("preview invariant: {error}"))?;
    println!(
        "layout size={}x{} items={} revision={} weight_g={}",
        preview.width(),
        preview.height(),
        preview.items().len(),
        preview.revision(),
        preview
            .total_weight_g(&catalog)
            .map_err(|error| format!("preview weight: {error}"))?
    );
    Ok(())
}
