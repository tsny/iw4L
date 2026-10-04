use bevy::prelude::Resource;
use inventory::{
    Catalog, ContainerId, GridContainer, ItemDefId, ItemInstance, ItemInstanceId, ItemKey,
    MAX_CONDITION, MergeRequest, PlacementTarget, TransferAmount, TransferRequest, UseEffect,
    merge_between, transfer_between,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Backpack,
    Loot,
}

impl Pane {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Backpack => "Backpack",
            Self::Loot => "Loot container",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub pane: Pane,
    pub instance: ItemInstanceId,
    pub rotated: bool,
}

#[derive(Resource)]
pub struct LabState {
    pub catalog: Catalog,
    pub backpack: GridContainer,
    pub loot: GridContainer,
    pub selected: Option<Selection>,
    pub status: String,
}

impl LabState {
    pub fn seeded(catalog: Catalog) -> Result<Self, String> {
        catalog
            .check_invariants()
            .map_err(|error| error.to_string())?;
        let bandage = id(&catalog, "base:bandage")?;
        let medkit = id(&catalog, "base:medkit")?;
        let radio = id(&catalog, "base:radio_parts")?;
        let ammo = id(&catalog, "base:rifle_ammo")?;
        let mut backpack =
            GridContainer::new(ContainerId(1), 8, 6).map_err(|error| error.to_string())?;
        let mut loot =
            GridContainer::new(ContainerId(2), 6, 6).map_err(|error| error.to_string())?;

        insert(
            &mut backpack,
            ItemInstanceId(1),
            bandage,
            3,
            [0, 0],
            false,
            &catalog,
        )?;
        insert(
            &mut backpack,
            ItemInstanceId(2),
            medkit,
            1,
            [2, 0],
            false,
            &catalog,
        )?;
        insert(
            &mut backpack,
            ItemInstanceId(3),
            radio,
            1,
            [4, 0],
            false,
            &catalog,
        )?;
        insert(
            &mut loot,
            ItemInstanceId(4),
            ammo,
            2,
            [0, 0],
            false,
            &catalog,
        )?;
        insert(
            &mut loot,
            ItemInstanceId(5),
            bandage,
            1,
            [2, 0],
            false,
            &catalog,
        )?;

        Ok(Self {
            catalog,
            backpack,
            loot,
            selected: None,
            status: "Select an item, press R to rotate, then choose a cell or matching stack."
                .into(),
        })
    }

    pub fn container(&self, pane: Pane) -> &GridContainer {
        match pane {
            Pane::Backpack => &self.backpack,
            Pane::Loot => &self.loot,
        }
    }

    pub fn select_item(&mut self, pane: Pane, instance: ItemInstanceId) {
        if self
            .selected
            .is_some_and(|selected| selected.pane == pane && selected.instance == instance)
        {
            self.selected = None;
            self.status = "Selection cleared.".into();
            return;
        }
        let Some(placement) = self.container(pane).placement(instance) else {
            self.status = format!("Item instance {} is unavailable.", instance.0);
            return;
        };
        self.selected = Some(Selection {
            pane,
            instance,
            rotated: placement.rotated,
        });
        self.status = format!("Selected {} item {}.", pane.label(), instance.0);
    }

    pub fn rotate_selection(&mut self) {
        let Some(mut selected) = self.selected else {
            self.status = "Select an item before rotating.".into();
            return;
        };
        let Some(item) = self.container(selected.pane).item(selected.instance) else {
            self.selected = None;
            self.status = "The selected item is no longer available.".into();
            return;
        };
        let Some(definition) = self.catalog.definition(item.definition) else {
            self.status = "The selected item definition is unavailable.".into();
            return;
        };
        if !definition.rotatable {
            self.status = format!("{} cannot rotate.", definition.name);
            return;
        }
        selected.rotated = !selected.rotated;
        self.selected = Some(selected);
        self.status = format!(
            "{} preview {}.",
            definition.name,
            if selected.rotated {
                "rotated"
            } else {
                "unrotated"
            }
        );
    }

    pub fn activate_cell(&mut self, pane: Pane, x: u8, y: u8) {
        let Some(selected) = self.selected else {
            self.status = "Select an item first.".into();
            return;
        };
        let target = PlacementTarget {
            x,
            y,
            rotated: selected.rotated,
        };
        let result = if selected.pane == pane {
            let revision = self.container(pane).revision();
            match pane {
                Pane::Backpack => {
                    self.backpack
                        .move_within(selected.instance, revision, target, &self.catalog)
                }
                Pane::Loot => {
                    self.loot
                        .move_within(selected.instance, revision, target, &self.catalog)
                }
            }
            .map(|()| selected.instance)
        } else {
            let request = TransferRequest {
                expected_source_revision: self.container(selected.pane).revision(),
                expected_destination_revision: self.container(pane).revision(),
                instance: selected.instance,
                amount: TransferAmount::Whole,
                target,
            };
            match (selected.pane, pane) {
                (Pane::Backpack, Pane::Loot) => {
                    transfer_between(&mut self.backpack, &mut self.loot, request, &self.catalog)
                }
                (Pane::Loot, Pane::Backpack) => {
                    transfer_between(&mut self.loot, &mut self.backpack, request, &self.catalog)
                }
                _ => unreachable!("same-pane move handled above"),
            }
        };
        match result {
            Ok(instance) => {
                self.selected = None;
                self.status = format!("Placed item {} at {x},{y}.", instance.0);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub fn activate_item(&mut self, pane: Pane, destination: ItemInstanceId) {
        let Some(selected) = self.selected else {
            self.select_item(pane, destination);
            return;
        };
        if selected.pane == pane && selected.instance == destination {
            self.select_item(pane, destination);
            return;
        }
        let Some(source_item) = self.container(selected.pane).item(selected.instance) else {
            self.selected = None;
            self.status = "The selected item is no longer available.".into();
            return;
        };
        let quantity = source_item.quantity;
        let result = if selected.pane == pane {
            let revision = self.container(pane).revision();
            match pane {
                Pane::Backpack => self.backpack.merge_within(
                    selected.instance,
                    destination,
                    revision,
                    quantity,
                    &self.catalog,
                ),
                Pane::Loot => self.loot.merge_within(
                    selected.instance,
                    destination,
                    revision,
                    quantity,
                    &self.catalog,
                ),
            }
        } else {
            let request = MergeRequest {
                expected_source_revision: self.container(selected.pane).revision(),
                expected_destination_revision: self.container(pane).revision(),
                source_instance: selected.instance,
                destination_instance: destination,
                quantity,
            };
            match (selected.pane, pane) {
                (Pane::Backpack, Pane::Loot) => {
                    merge_between(&mut self.backpack, &mut self.loot, request, &self.catalog)
                }
                (Pane::Loot, Pane::Backpack) => {
                    merge_between(&mut self.loot, &mut self.backpack, request, &self.catalog)
                }
                _ => unreachable!("same-pane merge handled above"),
            }
        };
        match result {
            Ok(()) => {
                self.selected = None;
                self.status = format!("Merged into item {}.", destination.0);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.status = "Selection cleared.".into();
    }

    pub fn selected_detail(&self) -> String {
        let Some(selected) = self.selected else {
            return "Nothing selected".into();
        };
        let Some(item) = self.container(selected.pane).item(selected.instance) else {
            return "Selected item unavailable".into();
        };
        let Some(definition) = self.catalog.definition(item.definition) else {
            return "Selected definition unavailable".into();
        };
        let footprint = definition.footprint.oriented(selected.rotated);
        let effect = match definition.use_effect {
            None => "no use effect".to_owned(),
            Some(UseEffect::Heal { amount }) => format!("heal {amount}"),
            Some(UseEffect::RestoreHeldAmmo { amount }) => format!("restore {amount} ammo"),
        };
        format!(
            "{} · {} · {}x{} · qty {} · {} g each · {}",
            definition.name,
            definition.key,
            footprint.width,
            footprint.height,
            item.quantity,
            definition.weight_g,
            effect
        )
    }

    pub fn check(&self) -> Result<String, String> {
        self.catalog
            .check_invariants()
            .map_err(|error| error.to_string())?;
        self.backpack
            .check_invariants(&self.catalog)
            .map_err(|error| error.to_string())?;
        self.loot
            .check_invariants(&self.catalog)
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "catalog={} digest={:016x} backpack={}x{}:{} loot={}x{}:{} weight_g={}/{}",
            self.catalog.definitions().len(),
            self.catalog.digest(),
            self.backpack.width(),
            self.backpack.height(),
            self.backpack.items().len(),
            self.loot.width(),
            self.loot.height(),
            self.loot.items().len(),
            self.backpack
                .total_weight_g(&self.catalog)
                .map_err(|error| error.to_string())?,
            self.loot
                .total_weight_g(&self.catalog)
                .map_err(|error| error.to_string())?,
        ))
    }
}

fn id(catalog: &Catalog, key: &str) -> Result<ItemDefId, String> {
    let key = ItemKey::parse(key).map_err(|error| error.to_string())?;
    catalog
        .id_for_key(&key)
        .ok_or_else(|| format!("base catalog missing `{key}`"))
}

fn insert(
    container: &mut GridContainer,
    instance: ItemInstanceId,
    definition: ItemDefId,
    quantity: u16,
    [x, y]: [u8; 2],
    rotated: bool,
    catalog: &Catalog,
) -> Result<(), String> {
    container
        .insert(
            ItemInstance {
                id: instance,
                definition,
                quantity,
                condition: MAX_CONDITION,
            },
            PlacementTarget { x, y, rotated },
            catalog,
        )
        .map_err(|error| error.to_string())
}
