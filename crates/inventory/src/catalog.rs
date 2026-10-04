use std::collections::BTreeMap;

use asset_core::{AssetKey, AssetKind};
use serde::Deserialize;

pub const ITEM_CATALOG_SCHEMA: u32 = 1;
pub const MAX_ITEM_DEFINITIONS: usize = u16::MAX as usize;
pub const MAX_ITEM_KEY_BYTES: usize = 64;
pub const MAX_ITEM_NAME_BYTES: usize = 96;
pub const MAX_FOOTPRINT: u8 = 16;
pub const MAX_ITEM_STACK: u16 = 10_000;
pub const MAX_ITEM_WEIGHT_G: u32 = 100_000_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemKey(String);

impl ItemKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, CatalogError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ITEM_KEY_BYTES {
            return Err(CatalogError::InvalidItemKey(value));
        }
        let Some((namespace, name)) = value.split_once(':') else {
            return Err(CatalogError::InvalidItemKey(value));
        };
        if namespace.is_empty()
            || name.is_empty()
            || name.contains(':')
            || !namespace.bytes().all(key_byte)
            || !name.bytes().all(key_byte)
        {
            return Err(CatalogError::InvalidItemKey(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ItemKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn key_byte(value: u8) -> bool {
    value.is_ascii_lowercase()
        || value.is_ascii_digit()
        || matches!(value, b'_' | b'-' | b'.' | b'/')
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemDefId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub width: u8,
    pub height: u8,
}

impl Footprint {
    pub fn new(width: u8, height: u8) -> Result<Self, CatalogError> {
        if width == 0 || height == 0 || width > MAX_FOOTPRINT || height > MAX_FOOTPRINT {
            return Err(CatalogError::InvalidFootprint([width, height]));
        }
        Ok(Self { width, height })
    }

    pub fn oriented(self, rotated: bool) -> Self {
        if rotated {
            Self {
                width: self.height,
                height: self.width,
            }
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseEffect {
    Heal { amount: u16 },
    RestoreHeldAmmo { amount: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemDefinition {
    pub id: ItemDefId,
    pub key: ItemKey,
    pub name: String,
    pub footprint: Footprint,
    pub rotatable: bool,
    pub max_stack: u16,
    pub weight_g: u32,
    pub icon: Option<AssetKey>,
    pub world_model: Option<AssetKey>,
    pub use_effect: Option<UseEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    schema: u32,
    definitions: Vec<ItemDefinition>,
    by_key: BTreeMap<ItemKey, ItemDefId>,
    canonical: Vec<u8>,
    digest: u64,
}

impl Catalog {
    pub fn from_json(source: &str) -> Result<Self, CatalogError> {
        let authored: AuthoredCatalog =
            serde_json::from_str(source).map_err(|error| CatalogError::Json(error.to_string()))?;
        Self::from_authored(authored)
    }

    fn from_authored(authored: AuthoredCatalog) -> Result<Self, CatalogError> {
        if authored.schema != ITEM_CATALOG_SCHEMA {
            return Err(CatalogError::UnknownSchema(authored.schema));
        }
        if authored.items.is_empty() || authored.items.len() > MAX_ITEM_DEFINITIONS {
            return Err(CatalogError::InvalidDefinitionCount(authored.items.len()));
        }

        let mut rows = Vec::with_capacity(authored.items.len());
        for row in authored.items {
            rows.push(ValidatedAuthoredItem::validate(row)?);
        }
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        for pair in rows.windows(2) {
            if pair[0].key == pair[1].key {
                return Err(CatalogError::DuplicateItemKey(pair[0].key.clone()));
            }
        }

        let mut definitions = Vec::with_capacity(rows.len());
        let mut by_key = BTreeMap::new();
        for (index, row) in rows.into_iter().enumerate() {
            let id = ItemDefId((index + 1) as u16);
            by_key.insert(row.key.clone(), id);
            definitions.push(ItemDefinition {
                id,
                key: row.key,
                name: row.name,
                footprint: row.footprint,
                rotatable: row.rotatable,
                max_stack: row.max_stack,
                weight_g: row.weight_g,
                icon: row.icon,
                world_model: row.world_model,
                use_effect: row.use_effect,
            });
        }
        let canonical = canonical_catalog(authored.schema, &definitions);
        let digest = fnv1a64(&canonical);
        let catalog = Self {
            schema: authored.schema,
            definitions,
            by_key,
            canonical,
            digest,
        };
        catalog.check_invariants()?;
        Ok(catalog)
    }

    pub fn schema(&self) -> u32 {
        self.schema
    }

    pub fn definitions(&self) -> &[ItemDefinition] {
        &self.definitions
    }

    pub fn definition(&self, id: ItemDefId) -> Option<&ItemDefinition> {
        id.0.checked_sub(1)
            .and_then(|index| self.definitions.get(index as usize))
    }

    pub fn definition_by_key(&self, key: &ItemKey) -> Option<&ItemDefinition> {
        self.by_key.get(key).and_then(|id| self.definition(*id))
    }

    pub fn id_for_key(&self, key: &ItemKey) -> Option<ItemDefId> {
        self.by_key.get(key).copied()
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    pub fn digest(&self) -> u64 {
        self.digest
    }

    pub fn check_invariants(&self) -> Result<(), CatalogError> {
        if self.schema != ITEM_CATALOG_SCHEMA
            || self.definitions.is_empty()
            || self.definitions.len() > MAX_ITEM_DEFINITIONS
            || self.by_key.len() != self.definitions.len()
        {
            return Err(CatalogError::InternalInvariant);
        }
        let mut previous: Option<&ItemKey> = None;
        for (index, definition) in self.definitions.iter().enumerate() {
            if definition.id.0 as usize != index + 1
                || previous.is_some_and(|key| key >= &definition.key)
                || self.by_key.get(&definition.key) != Some(&definition.id)
                || definition.name.is_empty()
                || definition.name.len() > MAX_ITEM_NAME_BYTES
                || definition.max_stack == 0
                || definition.max_stack > MAX_ITEM_STACK
                || definition.weight_g == 0
                || definition.weight_g > MAX_ITEM_WEIGHT_G
            {
                return Err(CatalogError::InternalInvariant);
            }
            Footprint::new(definition.footprint.width, definition.footprint.height)?;
            previous = Some(&definition.key);
        }
        if canonical_catalog(self.schema, &self.definitions) != self.canonical
            || fnv1a64(&self.canonical) != self.digest
        {
            return Err(CatalogError::InternalInvariant);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    Json(String),
    UnknownSchema(u32),
    InvalidDefinitionCount(usize),
    InvalidItemKey(String),
    DuplicateItemKey(ItemKey),
    InvalidName(ItemKey),
    InvalidFootprint([u8; 2]),
    InvalidStackLimit {
        key: ItemKey,
        value: u16,
    },
    InvalidWeight {
        key: ItemKey,
        value: u32,
    },
    InvalidAssetKey {
        key: ItemKey,
        field: &'static str,
    },
    WrongAssetKind {
        key: ItemKey,
        field: &'static str,
        expected: AssetKind,
        actual: AssetKind,
    },
    InvalidEffectAmount(ItemKey),
    InternalInvariant,
}

impl core::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "item catalog JSON: {error}"),
            Self::UnknownSchema(schema) => write!(f, "unsupported item catalog schema {schema}"),
            Self::InvalidDefinitionCount(count) => {
                write!(f, "item catalog has invalid definition count {count}")
            }
            Self::InvalidItemKey(key) => write!(f, "invalid item key `{key}`"),
            Self::DuplicateItemKey(key) => write!(f, "duplicate item key `{key}`"),
            Self::InvalidName(key) => write!(f, "item `{key}` has an invalid name"),
            Self::InvalidFootprint([width, height]) => {
                write!(f, "invalid item footprint {width}x{height}")
            }
            Self::InvalidStackLimit { key, value } => {
                write!(f, "item `{key}` has invalid max_stack {value}")
            }
            Self::InvalidWeight { key, value } => {
                write!(f, "item `{key}` has invalid weight_g {value}")
            }
            Self::InvalidAssetKey { key, field } => {
                write!(f, "item `{key}` has invalid {field} asset key")
            }
            Self::WrongAssetKind {
                key,
                field,
                expected,
                actual,
            } => write!(
                f,
                "item `{key}` {field} is {}, expected {}",
                actual.as_str(),
                expected.as_str()
            ),
            Self::InvalidEffectAmount(key) => {
                write!(f, "item `{key}` has a zero use-effect amount")
            }
            Self::InternalInvariant => f.write_str("item catalog invariant failed"),
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredCatalog {
    schema: u32,
    items: Vec<AuthoredItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredItem {
    key: String,
    name: String,
    footprint: [u8; 2],
    #[serde(default)]
    rotatable: bool,
    max_stack: u16,
    weight_g: u32,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    world_model: Option<String>,
    #[serde(default, rename = "use")]
    use_effect: Option<AuthoredUseEffect>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AuthoredUseEffect {
    Heal { amount: u16 },
    RestoreHeldAmmo { amount: u16 },
}

struct ValidatedAuthoredItem {
    key: ItemKey,
    name: String,
    footprint: Footprint,
    rotatable: bool,
    max_stack: u16,
    weight_g: u32,
    icon: Option<AssetKey>,
    world_model: Option<AssetKey>,
    use_effect: Option<UseEffect>,
}

impl ValidatedAuthoredItem {
    fn validate(row: AuthoredItem) -> Result<Self, CatalogError> {
        let key = ItemKey::parse(row.key)?;
        let name = row.name.trim();
        if name.is_empty() || name.len() > MAX_ITEM_NAME_BYTES || name != row.name {
            return Err(CatalogError::InvalidName(key));
        }
        let footprint = Footprint::new(row.footprint[0], row.footprint[1])?;
        if row.max_stack == 0 || row.max_stack > MAX_ITEM_STACK {
            return Err(CatalogError::InvalidStackLimit {
                key,
                value: row.max_stack,
            });
        }
        if row.weight_g == 0 || row.weight_g > MAX_ITEM_WEIGHT_G {
            return Err(CatalogError::InvalidWeight {
                key,
                value: row.weight_g,
            });
        }
        let icon = parse_asset(&key, "icon", row.icon, AssetKind::Material)?;
        let world_model = parse_asset(&key, "world_model", row.world_model, AssetKind::XModel)?;
        let use_effect = match row.use_effect {
            Some(AuthoredUseEffect::Heal { amount }) if amount > 0 => {
                Some(UseEffect::Heal { amount })
            }
            Some(AuthoredUseEffect::RestoreHeldAmmo { amount }) if amount > 0 => {
                Some(UseEffect::RestoreHeldAmmo { amount })
            }
            Some(_) => return Err(CatalogError::InvalidEffectAmount(key)),
            None => None,
        };
        Ok(Self {
            key,
            name: row.name,
            footprint,
            rotatable: row.rotatable,
            max_stack: row.max_stack,
            weight_g: row.weight_g,
            icon,
            world_model,
            use_effect,
        })
    }
}

fn parse_asset(
    item: &ItemKey,
    field: &'static str,
    value: Option<String>,
    expected: AssetKind,
) -> Result<Option<AssetKey>, CatalogError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let asset = AssetKey::parse(&value).map_err(|_| CatalogError::InvalidAssetKey {
        key: item.clone(),
        field,
    })?;
    if asset.kind != expected {
        return Err(CatalogError::WrongAssetKind {
            key: item.clone(),
            field,
            expected,
            actual: asset.kind,
        });
    }
    Ok(Some(asset))
}

fn canonical_catalog(schema: u32, definitions: &[ItemDefinition]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, schema);
    put_u32(&mut out, definitions.len() as u32);
    for definition in definitions {
        put_u16(&mut out, definition.id.0);
        put_text(&mut out, definition.key.as_str());
        put_text(&mut out, &definition.name);
        out.push(definition.footprint.width);
        out.push(definition.footprint.height);
        out.push(u8::from(definition.rotatable));
        put_u16(&mut out, definition.max_stack);
        put_u32(&mut out, definition.weight_g);
        put_asset(&mut out, definition.icon.as_ref());
        put_asset(&mut out, definition.world_model.as_ref());
        match definition.use_effect {
            None => out.push(0),
            Some(UseEffect::Heal { amount }) => {
                out.push(1);
                put_u16(&mut out, amount);
            }
            Some(UseEffect::RestoreHeldAmmo { amount }) => {
                out.push(2);
                put_u16(&mut out, amount);
            }
        }
    }
    out
}

fn put_asset(out: &mut Vec<u8>, asset: Option<&AssetKey>) {
    match asset {
        Some(asset) => {
            out.push(1);
            put_text(out, &asset.display());
        }
        None => out.push(0),
    }
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_u32(out, text.len() as u32);
    out.extend_from_slice(text.as_bytes());
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
