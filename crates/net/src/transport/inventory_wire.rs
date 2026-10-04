use std::collections::BTreeMap;

use sim::{ClientId, ContainerId, PlayerInventory, SnapshotMeta};

use crate::transport::wire::{WireError, WireReader, WireWriter};

const MAX_INVENTORY_CONTAINERS: usize = 64;
const MAX_CONTAINER_ITEMS: usize = 256;

#[derive(Debug, Default)]
pub struct InventorySyncEncoder {
    baseline: BTreeMap<ContainerId, (ClientId, PlayerInventory)>,
}

impl InventorySyncEncoder {
    pub fn reset(&mut self) {
        self.baseline.clear();
    }

    pub fn adopt_baseline(&mut self, meta: &SnapshotMeta) {
        self.baseline = inventory_rows(meta);
    }

    pub fn encode(&mut self, meta: &SnapshotMeta) -> Vec<u8> {
        let current = inventory_rows(meta);
        let mut out = WireWriter::with_capacity(32);
        debug_assert!(current.len() <= MAX_INVENTORY_CONTAINERS);
        out.put_u16(current.len() as u16);
        for (container, (owner, inventory)) in &current {
            out.put_u32(owner.0);
            out.put_u32(container.0);
            out.put_u32(inventory.backpack().revision());
            let unchanged = matches!(
                self.baseline.get(container),
                Some((baseline_owner, baseline))
                    if baseline_owner == owner && baseline == inventory
            );
            if unchanged {
                out.put_u8(0);
            } else {
                out.put_u8(1);
                encode_inventory_rows(&mut out, inventory);
            }
        }
        self.baseline = current;
        out.finish()
    }
}

#[derive(Debug, Default)]
pub struct InventorySyncDecoder {
    state: BTreeMap<ContainerId, (ClientId, PlayerInventory)>,
}

impl InventorySyncDecoder {
    pub fn reset(&mut self) {
        self.state.clear();
    }

    pub fn adopt_baseline(&mut self, meta: &SnapshotMeta) {
        self.state = inventory_rows(meta);
    }

    pub fn apply_wire(
        &mut self,
        wire: &[u8],
        clients: &mut [(ClientId, sim::ClientSnapshotMeta)],
    ) -> Result<(), WireError> {
        let mut input = WireReader::new(wire);
        let count = usize::from(input.get_u16()?);
        if count > MAX_INVENTORY_CONTAINERS {
            return Err(WireError::Malformed(
                "inventory container count exceeds limit",
            ));
        }
        let mut next = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for _ in 0..count {
            let owner = ClientId(input.get_u32()?);
            let container = ContainerId(input.get_u32()?);
            let revision = input.get_u32()?;
            if next.contains_key(&container) || owners.insert(owner, container).is_some() {
                return Err(WireError::Malformed("duplicate inventory container"));
            }
            let inventory = match input.get_u8()? {
                0 => {
                    let Some((baseline_owner, baseline)) = self.state.get(&container) else {
                        return Err(WireError::Malformed(
                            "unchanged inventory container has no baseline",
                        ));
                    };
                    if *baseline_owner != owner || baseline.backpack().revision() != revision {
                        return Err(WireError::Malformed(
                            "unchanged inventory container mismatches baseline",
                        ));
                    }
                    baseline.clone()
                }
                1 => decode_inventory_rows(&mut input, owner, container, revision)?,
                _ => return Err(WireError::Malformed("bad inventory container tag")),
            };
            next.insert(container, (owner, inventory));
        }
        if !input.is_empty() {
            return Err(WireError::Malformed("trailing bytes after inventory sync"));
        }
        for (owner, _) in next.values() {
            if !clients.iter().any(|(client, _)| client == owner) {
                return Err(WireError::Malformed(
                    "inventory owner is absent from snapshot",
                ));
            }
        }
        for (_, meta) in clients.iter_mut() {
            meta.inventory = None;
        }
        for (owner, inventory) in next.values() {
            let meta = clients
                .iter_mut()
                .find(|(client, _)| client == owner)
                .map(|(_, meta)| meta)
                .ok_or(WireError::Malformed(
                    "inventory owner is absent from snapshot",
                ))?;
            meta.inventory = Some(inventory.clone());
        }
        self.state = next;
        Ok(())
    }
}

fn inventory_rows(meta: &SnapshotMeta) -> BTreeMap<ContainerId, (ClientId, PlayerInventory)> {
    meta.clients
        .iter()
        .filter_map(|(owner, meta)| {
            meta.inventory
                .as_ref()
                .map(|inventory| (inventory.backpack().id(), (*owner, inventory.clone())))
        })
        .collect()
}

fn encode_inventory_rows(out: &mut WireWriter, inventory: &PlayerInventory) {
    out.put_u32(inventory.next_serial());
    let backpack = inventory.backpack();
    debug_assert!(backpack.items().len() <= MAX_CONTAINER_ITEMS);
    out.put_u16(backpack.items().len() as u16);
    for item in backpack.items() {
        let placement = backpack
            .placement(item.id)
            .expect("validated inventory item must have a placement");
        out.put_u64(item.id.0);
        out.put_u16(item.definition.0);
        out.put_u16(item.quantity);
        out.put_u16(item.condition);
        out.put_u8(placement.x);
        out.put_u8(placement.y);
        out.put_u8(u8::from(placement.rotated));
    }
    match inventory.latest_notice() {
        None => out.put_u8(0),
        Some(notice) => {
            out.put_u8(1);
            out.put_u32(notice.request_id);
            out.put_u16(notice.definition.0);
            out.put_u16(notice.quantity);
            out.put_u32(notice.revision);
        }
    }
}

fn decode_inventory_rows(
    input: &mut WireReader<'_>,
    owner: ClientId,
    container: ContainerId,
    revision: u32,
) -> Result<PlayerInventory, WireError> {
    let next_serial = input.get_u32()?;
    let count = usize::from(input.get_u16()?);
    if count > MAX_CONTAINER_ITEMS {
        return Err(WireError::Malformed("inventory item count exceeds limit"));
    }
    let mut items = Vec::with_capacity(count);
    let mut placements = Vec::with_capacity(count);
    for _ in 0..count {
        let id = sim::ItemInstanceId(input.get_u64()?);
        items.push(sim::ItemInstance {
            id,
            definition: sim::ItemDefId(input.get_u16()?),
            quantity: input.get_u16()?,
            condition: input.get_u16()?,
        });
        placements.push(sim::Placement {
            instance: id,
            x: input.get_u8()?,
            y: input.get_u8()?,
            rotated: match input.get_u8()? {
                0 => false,
                1 => true,
                _ => return Err(WireError::Malformed("bad inventory rotation tag")),
            },
        });
    }
    let latest_notice = match input.get_u8()? {
        0 => None,
        1 => Some(sim::InventoryNotice {
            request_id: input.get_u32()?,
            definition: sim::ItemDefId(input.get_u16()?),
            quantity: input.get_u16()?,
            revision: input.get_u32()?,
        }),
        _ => return Err(WireError::Malformed("bad inventory notice tag")),
    };
    let inventory = PlayerInventory::from_rows(
        owner,
        revision,
        next_serial,
        items,
        placements,
        latest_notice,
    )
    .map_err(|_| WireError::Malformed("invalid inventory container"))?;
    if inventory.backpack().id() != container {
        return Err(WireError::Malformed("inventory container owner mismatch"));
    }
    Ok(inventory)
}
