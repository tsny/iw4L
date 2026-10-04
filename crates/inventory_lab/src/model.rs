use bevy::prelude::Resource;
use inventory::{
    Catalog, ContainerId, GridContainer, ItemDefId, ItemInstance, ItemInstanceId, ItemKey,
    MAX_CONDITION, PlacementTarget,
};
use inventory_ui::{
    DropAction, DropResult, InventoryOverlay, InventoryPane, InventoryView, apply_action,
};

#[derive(Resource)]
pub struct LabState {
    pub catalog: Catalog,
    pub backpack: GridContainer,
    pub loot: Option<GridContainer>,
}

impl LabState {
    pub fn seeded(catalog: Catalog) -> Result<Self, String> {
        catalog.check_invariants().map_err(|e| e.to_string())?;
        let (bandage, medkit, radio, ammo) = (
            id(&catalog, "base:bandage")?,
            id(&catalog, "base:medkit")?,
            id(&catalog, "base:radio_parts")?,
            id(&catalog, "base:rifle_ammo")?,
        );
        let mut backpack = GridContainer::new(ContainerId(1), 8, 6).map_err(|e| e.to_string())?;
        let mut loot = GridContainer::new(ContainerId(2), 6, 6).map_err(|e| e.to_string())?;
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
            loot: Some(loot),
        })
    }

    pub fn view(&self) -> InventoryView {
        InventoryView {
            catalog: self.catalog.clone(),
            player: self.backpack.clone(),
            container: self.loot.clone(),
        }
    }

    pub fn apply(&mut self, action: DropAction) -> Result<(), String> {
        inventory_ui::apply_action(&self.catalog, &mut self.backpack, &mut self.loot, action)
    }

    pub fn check(&self) -> Result<String, String> {
        self.catalog.check_invariants().map_err(|e| e.to_string())?;
        self.backpack
            .check_invariants(&self.catalog)
            .map_err(|e| e.to_string())?;
        let loot = self.loot.as_ref().ok_or("loot container missing")?;
        loot.check_invariants(&self.catalog)
            .map_err(|e| e.to_string())?;
        let view = self.view();
        let mut overlay = InventoryOverlay::default();
        overlay.begin_drag(&view, InventoryPane::Container, ItemInstanceId(4))?;
        overlay.hover_cell(InventoryPane::Player, 6, 0);
        let request = overlay.complete_drop(&view)?;
        if overlay.pending_count() != 1 || overlay.complete_drop(&view).is_ok() {
            return Err("completed drag did not emit exactly one pending transaction".into());
        }
        let mut preview_player = self.backpack.clone();
        let mut preview_loot = self.loot.clone();
        apply_action(
            &self.catalog,
            &mut preview_player,
            &mut preview_loot,
            request.action,
        )?;
        overlay.resolve(&DropResult {
            id: request.id,
            accepted: true,
            detail: "accepted".into(),
        });
        if overlay.pending_count() != 0 {
            return Err("accepted transaction remained pending".into());
        }
        Ok(format!(
            "catalog={} digest={:016x} backpack={}x{}:{} loot={}x{}:{} weight_g={}/{} ui_drop=one pending=cleared",
            self.catalog.definitions().len(),
            self.catalog.digest(),
            self.backpack.width(),
            self.backpack.height(),
            self.backpack.items().len(),
            loot.width(),
            loot.height(),
            loot.items().len(),
            self.backpack
                .total_weight_g(&self.catalog)
                .map_err(|e| e.to_string())?,
            loot.total_weight_g(&self.catalog)
                .map_err(|e| e.to_string())?
        ))
    }
}

fn id(catalog: &Catalog, key: &str) -> Result<ItemDefId, String> {
    let key = ItemKey::parse(key).map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())
}
