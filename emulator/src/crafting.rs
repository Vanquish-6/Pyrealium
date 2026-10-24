//! Bounded native crafting requests and a two-recipe runtime profile.
//!
//! Captures: crafting.item.2.elog and crafting/ac2_craft_assembling_multiple_
//! items_into_one.elog. Native ingredient hashing is corroborated by 00460040
//! and PHashTable<unsigned_long,InstanceID>::StreamPack 004431d0. Completion
//! reaches the 7148 ingredient UI (004696a0 / 00418780). Capture build identity
//! is unknown; full crafting entitlement, cost, XP and chance rules are not
//! inferred. The caller supplies explicit server-owned development authorization.
use crate::{
    dat::Dat,
    inventory::{BAG_SLOTS, Inventory, Item, Location},
    inventory_request::Metadata,
    world::WorldError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub const VALIDATE: u32 = 0x82060006;
pub const EXECUTE: u32 = 0x82060012;
pub const VALIDATION_REPLY: u32 = 0x82060016;
pub const COMPLETION: u32 = 0x8206001c;
pub const INITIALIZE: u32 = 0x82060019;
pub const DEFAULT_SKILLS: u32 = 0x82060034;
pub const HEALTH_RECIPE: u32 = 0x6f001f17;
pub const VIGOR_RECIPE: u32 = 0x6f001f18;
pub const CRAFT_SKILL_DID: u32 = 0x6f0028d7;
pub const MUCOR: u32 = 0x47001744;
pub const CATALYST: u32 = 0x470027a7;
/// English StringInfo 2500797e / a8607cec identifies this ingredient as Tallow.
pub const TALLOW: u32 = CATALYST;
/// Recipe ingredient package 01d4 field 12; native 01d4000c at 0xc9558
/// reads this requirement and 01d40006 compares it with item quantity 0106.
pub const TALLOW_QUANTITY: u16 = 100;
pub const HEALTH_OUTPUT: u32 = 0x47001745;
pub const VIGOR_OUTPUT: u32 = 0x47001746;
pub const MAX_INGREDIENTS: usize = 16;
pub const COOLDOWN_MS: u64 = 1000; // Explicit development policy.

/// Validate the interpreted layouts used by the native crafting panel before
/// emitting its initialization. Native 7148 PackBuffer supplies the graph;
/// these interpreted package field counts come from its WLib package metadata.
pub fn validate_wlib(bytes: &[u8]) -> Result<(), WorldError> {
    crate::ui::validate_layouts(bytes, &[(0x1d2, 7), (0x345f, 6), (0x1d9, 12)])
}

struct PanelObject {
    kind: u32,
    native: Vec<u32>,
    fields: Vec<(u32, u8)>,
}
impl PanelObject {
    fn interpreted(package: u32, fields: &[(u32, u8)]) -> Self {
        Self {
            kind: package << 16,
            native: Vec::new(),
            fields: fields.to_vec(),
        }
    }
    fn native(package: u32, words: &[u32]) -> Self {
        Self {
            kind: 0xffff0000 | package,
            native: words.to_vec(),
            fields: Vec::new(),
        }
    }
}
fn panel_graph(objects: &[PanelObject]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let word = |bytes: &mut Vec<u8>, value: u32| bytes.extend(value.to_le_bytes());
    word(&mut bytes, 5);
    word(&mut bytes, objects.len() as u32);
    for id in 1..=objects.len() {
        word(&mut bytes, id as u32);
    }
    for object in objects {
        for value in [0x44000100, 0, object.kind] {
            word(&mut bytes, value);
        }
        for value in &object.native {
            word(&mut bytes, *value);
        }
        word(&mut bytes, object.fields.len() as u32);
        for (value, _) in &object.fields {
            word(&mut bytes, *value);
        }
        for (_, tag) in &object.fields {
            bytes.push(*tag);
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
    bytes
}
fn panel_skill() -> PanelObject {
    PanelObject::interpreted(
        0x345f,
        &[
            (0, 0x82),
            (0, 0xcd),
            (CRAFT_SKILL_DID, 2),
            (0, 0x82),
            (0, 0xcd),
            (1, 2),
        ],
    )
}
fn panel_recipe(did: u32) -> PanelObject {
    // Observed default recipe-state values: no charge/level/time overrides.
    // Three doubles use native -1.0 sentinels, not captured player timestamps.
    PanelObject::interpreted(
        0x1d9,
        &[
            (0, 2),
            (0, 0xc3),
            (0xbff00000, 0xcd),
            (0, 2),
            (0, 0xc3),
            (0xbff00000, 0xcd),
            (u32::MAX, 2),
            (did, 2),
            (0, 0xc3),
            (0xbff00000, 0xcd),
            (0, 2),
            (u32::MAX, 2),
        ],
    )
}

/// Native crafting-panel initialization, sent after UI_Load. Call this again
/// after granting training-kit authorization, using its authoritative value.
///
/// Retained ac2_login_from_charoverview.elog corroborates both message graphs,
/// their package layouts and the level-one 6f0028d7 entry. Capture build identity
/// is unknown. This fresh graph grants only the two development potion recipes;
/// it does not replay the captured hundreds of recipes, XP or last-craft time.
/// The root's XP modifier, count and time are zero: crafting XP is not awarded
/// by this development profile. Original entitlement, cost and XP rules remain
/// absent; the recovered one-Mucor/100-Tallow quantities are enforced separately.
pub fn initialization(authorized: bool) -> Result<Vec<Vec<u8>>, WorldError> {
    let default_objects = if authorized {
        vec![PanelObject::native(0x65, &[1, 2]), panel_skill()]
    } else {
        vec![PanelObject::native(0x65, &[0])]
    };
    let mut objects = vec![PanelObject::interpreted(
        0x1d2,
        &[
            (u32::MAX, 1),
            (0, 3),
            (0, 4),
            (0, 0x83),
            (0, 0xcd),
            (2, 1),
            (if authorized { 4 } else { 3 }, 1),
        ],
    )];
    if authorized {
        objects.extend([
            PanelObject::native(0x11, &[1, CRAFT_SKILL_DID, 3]),
            panel_skill(),
            PanelObject::native(0x11, &[2, HEALTH_RECIPE, 5, VIGOR_RECIPE, 6]),
            panel_recipe(HEALTH_RECIPE),
            panel_recipe(VIGOR_RECIPE),
        ]);
    } else {
        objects.extend([
            PanelObject::native(0x11, &[0]),
            PanelObject::native(0x11, &[0]),
        ]);
    }
    Ok(vec![
        crate::script::private_call(DEFAULT_SKILLS, &panel_graph(&default_objects))?,
        crate::script::private_call(INITIALIZE, &panel_graph(&objects))?,
    ])
}

/// Asset identity and ingredient quantities come from original DAT data.
/// Tallow is provided only as an exact 100-unit bundle; the runtime stack limit
/// of 100 is bounded development policy (the DAT maximum is 1,000,000).
/// Texture icons and appearance animations are distinct assets.
#[derive(Clone, Copy, Debug)]
pub struct ItemProfile {
    pub definition: u32,
    pub visual: u32,
    pub physical: u32,
    pub appearance: u32,
    pub texture: u32,
}

pub fn profile(definition: u32) -> Option<ItemProfile> {
    let (visual, physical, appearance, texture) = match definition {
        MUCOR => (0x1f000e37, 0x4700176b, 0x20000021, 0x4100034b),
        TALLOW => (0x1f000234, 0x47000260, 0, 0x410082d6),
        HEALTH_OUTPUT => (0x1f000e3f, 0x47001773, 0x20000268, 0x41006f81),
        VIGOR_OUTPUT => (0x1f000e40, 0x47001774, 0x20000268, 0x41006f82),
        _ => return None,
    };
    Some(ItemProfile {
        definition,
        visual,
        physical,
        appearance,
        texture,
    })
}

pub fn item(definition: u32, id: u64, slot: u16) -> Result<Item, WorldError> {
    let profile = profile(definition).ok_or(WorldError::InvalidValue)?;
    if [0, u64::MAX].contains(&id) || slot >= BAG_SLOTS {
        return Err(WorldError::InvalidValue);
    }
    Ok(Item {
        id,
        definition,
        visual: profile.visual,
        quantity: quantity(definition),
        stack_limit: quantity(definition),
        equipment_mask: 0,
        location: Location::Bag(slot),
    })
}

fn quantity(definition: u32) -> u16 {
    if definition == TALLOW {
        TALLOW_QUANTITY
    } else {
        1
    }
}

fn canonical_ingredient(item: &Item, expected: u32) -> bool {
    profile(expected).is_some_and(|profile| {
        item.definition == expected
            && item.visual == profile.visual
            && item.quantity == quantity(expected)
            && item.stack_limit == quantity(expected)
            && item.equipment_mask == 0
            && matches!(item.location, Location::Bag(_))
    })
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// Exact asset profiles, without broadening the generic qualities parser.
/// Metadata's historical `icon` member is acquisition appearance field 6,
/// not gmWeenieDesc::_icon. Tallow has no quality 103: its appearance is zero.
/// Retained stacked-Tallow acquisition messages also carry zero appearance.
pub fn metadata(portal: &mut Dat, definition: u32) -> io::Result<Metadata> {
    let invalid = || io::Error::other("unsupported native crafting item profile");
    let profile = profile(definition).ok_or_else(invalid)?;
    if definition == MUCOR {
        return crate::merchants::validate_mucor(portal);
    }
    if definition == HEALTH_OUTPUT {
        return crate::item_services::validate_potion(portal);
    }
    let (length, hash, physical_hash) = match definition {
        TALLOW => (192, 0x29beb90770d61b34, 0x673c3de26bf871af),
        VIGOR_OUTPUT => (288, 0x11b0b9d6a9196bba, 0x1e0b933e6b4054f0),
        _ => return Err(invalid()),
    };
    let descriptor = portal.read(0x81000000 | (definition & 0xffffff))?;
    let physical = portal.read(profile.physical)?;
    if descriptor.len() != length
        || fingerprint(&descriptor) != hash
        || physical.len() != 16
        || fingerprint(&physical) != physical_hash
    {
        return Err(invalid());
    }
    portal.read(definition)?;
    portal.read(profile.visual)?;
    let texture = portal.read(profile.texture)?;
    let (texture_length, texture_hash) = if definition == TALLOW {
        (3096, 0x0b09d7fb11b16f48)
    } else {
        (4120, 0x43b152d0c9a82103)
    };
    if texture.len() != texture_length || fingerprint(&texture) != texture_hash {
        return Err(invalid());
    }
    if profile.appearance != 0 {
        portal.read(profile.appearance)?;
    }
    Ok(Metadata {
        icon: profile.appearance,
        physical: profile.physical,
        equipment: None,
    })
}

pub fn validate_tallow(portal: &mut Dat) -> io::Result<Metadata> {
    metadata(portal, TALLOW)
}

pub fn validate_vigor(portal: &mut Dat) -> io::Result<Metadata> {
    metadata(portal, VIGOR_OUTPUT)
}

pub fn tallow(id: u64, slot: u16) -> Item {
    Item {
        id,
        definition: TALLOW,
        visual: 0x1f000234,
        quantity: TALLOW_QUANTITY,
        stack_limit: TALLOW_QUANTITY,
        equipment_mask: 0,
        location: Location::Bag(slot),
    }
}

pub fn output(recipe_id: u32, id: u64, slot: u16) -> Option<Item> {
    item(recipe(recipe_id)?.output, id, slot).ok()
}

#[derive(Clone, Copy, Debug)]
pub struct Recipe {
    pub id: u32,
    pub craft_skill: u32,
    pub inputs: [u32; 2],
    pub output: u32,
}
pub fn recipe(id: u32) -> Option<Recipe> {
    let output = match id {
        HEALTH_RECIPE => HEALTH_OUTPUT,
        VIGOR_RECIPE => VIGOR_OUTPUT,
        _ => return None,
    };
    Some(Recipe {
        id,
        craft_skill: CRAFT_SKILL_DID,
        inputs: [MUCOR, CATALYST],
        output,
    })
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u32, WorldError> {
        let value = self
            .data
            .get(self.position..self.position + 4)
            .ok_or(WorldError::Truncated)?;
        self.position += 4;
        Ok(u32::from_le_bytes(value.try_into().unwrap()))
    }
    fn expect(&mut self, value: u32) -> Result<(), WorldError> {
        if self.word()? != value {
            return Err(WorldError::InvalidValue);
        }
        Ok(())
    }
    fn integer(&mut self) -> Result<u32, WorldError> {
        self.expect(1)?;
        self.word()
    }
    fn iid_raw(&mut self) -> Result<u64, WorldError> {
        let value = u64::from(self.word()?) | (u64::from(self.word()?) << 32);
        if [0, u64::MAX].contains(&value) {
            return Err(WorldError::InvalidValue);
        }
        Ok(value)
    }
    fn iid(&mut self) -> Result<u64, WorldError> {
        self.expect(2)?;
        self.iid_raw()
    }
    fn finish(&self) -> Result<(), WorldError> {
        if self.position != self.data.len() {
            Err(WorldError::TrailingBytes)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateRequest {
    pub item: u64,
    pub ordinal: u32,
    pub recipe: u32,
}
impl ValidateRequest {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        if arguments.len() > 36 {
            return Err(WorldError::TrailingBytes);
        }
        let mut r = Reader {
            data: arguments,
            position: 0,
        };
        if r.integer()? != 0 {
            return Err(WorldError::InvalidValue);
        }
        let value = Self {
            item: r.iid()?,
            ordinal: r.integer()?,
            recipe: r.integer()?,
        };
        r.finish()?;
        if !(1..=MAX_INGREDIENTS as u32).contains(&value.ordinal) || value.recipe >> 24 != 0x6f {
            return Err(WorldError::InvalidValue);
        }
        Ok(value)
    }
    pub fn allowed(&self, inventory: &Inventory, authorized: bool) -> bool {
        let Some(recipe) = recipe(self.recipe) else {
            return false;
        };
        let Some(expected) = self
            .ordinal
            .checked_sub(1)
            .and_then(|i| recipe.inputs.get(i as usize))
        else {
            return false;
        };
        authorized
            && inventory.dense_bags()
            && inventory
                .items()
                .any(|item| item.id == self.item && canonical_ingredient(item, *expected))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecuteRequest {
    pub primary: u64,
    pub ingredients: BTreeMap<u32, u64>,
    pub spinner: u32,
    pub recipe: u32,
}
impl ExecuteRequest {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        if arguments.len() > 60 + 12 * MAX_INGREDIENTS {
            return Err(WorldError::LimitExceeded);
        }
        let mut r = Reader {
            data: arguments,
            position: 0,
        };
        let primary = r.iid()?;
        r.expect(5)?;
        r.expect(1)?;
        if [0, u32::MAX].contains(&r.word()?) {
            return Err(WorldError::InvalidValue);
        }
        r.expect(0x44000100)?;
        r.expect(0)?;
        r.expect(0xffff0008)?;
        let header = r.word()?;
        let count = (header & 0xffffff) as usize;
        if header >> 24 > 22 || count == 0 || count > MAX_INGREDIENTS {
            return Err(WorldError::LimitExceeded);
        }
        let mut ingredients = BTreeMap::new();
        let mut identities = BTreeSet::new();
        for _ in 0..count {
            let ordinal = r.word()?;
            let item = r.iid_raw()?;
            if !(1..=MAX_INGREDIENTS as u32).contains(&ordinal)
                || ingredients.insert(ordinal, item).is_some()
                || !identities.insert(item)
            {
                return Err(WorldError::InvalidValue);
            }
        }
        r.expect(0)?; // Native ALHash has no interpreted HeapObject fields.
        let spinner = r.integer()?;
        let recipe = r.integer()?;
        r.finish()?;
        if spinner > 1000 || recipe >> 24 != 0x6f || !identities.contains(&primary) {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            primary,
            ingredients,
            spinner,
            recipe,
        })
    }
    /// Exact whole-bundle subset: one Mucor and 100 Tallow, matching native
    /// ingredient requirements. Partial-stack consumption remains unsupported.
    pub fn require_inputs(
        &self,
        inventory: &Inventory,
        authorized: bool,
    ) -> Result<[u64; 2], WorldError> {
        let definition = recipe(self.recipe).ok_or(WorldError::InvalidValue)?;
        if !authorized
            || self.spinner != 0
            || self.ingredients.len() != 2
            || !inventory.dense_bags()
        {
            return Err(WorldError::InvalidValue);
        }
        let mut result = [0; 2];
        for (index, expected) in definition.inputs.iter().enumerate() {
            let id = *self
                .ingredients
                .get(&(index as u32 + 1))
                .ok_or(WorldError::InvalidValue)?;
            if !inventory
                .items()
                .any(|item| item.id == id && canonical_ingredient(item, *expected))
            {
                return Err(WorldError::InvalidValue);
            }
            result[index] = id;
        }
        if result[0] == result[1] || !result.contains(&self.primary) {
            return Err(WorldError::InvalidValue);
        }
        Ok(result)
    }
}

pub fn select_inventory(
    id: u32,
    inventory: &Inventory,
    authorized: bool,
) -> Result<ExecuteRequest, WorldError> {
    let definition = recipe(id).ok_or(WorldError::InvalidValue)?;
    let mut ingredients = BTreeMap::new();
    for (index, expected) in definition.inputs.iter().enumerate() {
        let item = inventory
            .items()
            .find(|item| canonical_ingredient(item, *expected))
            .ok_or(WorldError::InvalidValue)?;
        ingredients.insert(index as u32 + 1, item.id);
    }
    let request = ExecuteRequest {
        primary: ingredients[&1],
        ingredients,
        spinner: 0,
        recipe: id,
    };
    request.require_inputs(inventory, authorized)?;
    Ok(request)
}

pub fn validation_reply(request: &ValidateRequest, accepted: bool) -> Result<Vec<u8>, WorldError> {
    let mut args = crate::script::integer(if accepted { 0 } else { 40001 });
    args.extend(crate::script::integer(0));
    args.extend(crate::script::identity(request.item));
    args.extend(crate::script::integer(request.ordinal));
    args.extend(crate::script::integer(request.recipe));
    crate::script::private_call(VALIDATION_REPLY, &args)
}

/// 40025 is observed in a failed craft completion; its precise localized error
/// meaning is unresolved. Callers must provide their own truthful chat reason.
pub fn completed(id: u32, success: bool) -> Result<Vec<u8>, WorldError> {
    if id >> 24 != 0x6f {
        return Err(WorldError::InvalidValue);
    }
    let mut args = crate::script::integer(1);
    args.extend(crate::script::integer(if success { 0 } else { 40025 }));
    args.extend(crate::script::integer(id));
    crate::script::private_call(COMPLETION, &args)
}

/// Pinned schema checks for the recovered two-ingredient potion graphs. This
/// establishes identities only; quantities, entitlement and costs stay explicit.
pub fn validate_assets(portal: &mut Dat) -> io::Result<()> {
    let invalid = || io::Error::other("unsupported native potion recipe graph");
    for (id, output) in [(HEALTH_RECIPE, HEALTH_OUTPUT), (VIGOR_RECIPE, VIGOR_OUTPUT)] {
        let data = portal.read(id)?;
        let hash = if id == HEALTH_RECIPE {
            0xfe785ca8fcbe9b88
        } else {
            0x93c3bd24e230ac1a
        };
        if data.len() != 1448 || fingerprint(&data) != hash {
            return Err(invalid());
        }
        let w = |o| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        if w(0) != id
            || w(4) != 1436
            || w(8) != 32
            || w(0x94) != 0x01d70000
            || w(0x98) != 37
            || w(0x238) != 5
            || w(0x244) != output
        {
            return Err(invalid());
        }
        for (offset, definition) in [(0x3e4, MUCOR), (0x4d8, CATALYST)] {
            if w(offset) != 0x44000100
                || w(offset + 8) != 0xffff0001
                || w(offset + 12) != 0x02000001
                || w(offset + 16) != definition
                || w(offset + 20) != definition
                || w(offset + 24) != 0
            {
                return Err(invalid());
            }
        }
    }
    for id in [
        MUCOR,
        CATALYST,
        HEALTH_OUTPUT,
        VIGOR_OUTPUT,
        CRAFT_SKILL_DID,
    ] {
        portal.read(id)?;
    }
    if fingerprint(&portal.read(CRAFT_SKILL_DID)?) != 0xc5454bfe119ba394 {
        return Err(invalid());
    }
    let _ = crafting_levels();
    Ok(())
}

/// One rank from Pyreal_DB_Insert_CraftingData.sql.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CraftRank {
    pub level: u16,
    pub curve: u32,
    pub learning_limit: u32,
    pub cxp: u32,
}

fn crafting_levels() -> &'static BTreeMap<u32, Vec<CraftRank>> {
    use std::sync::OnceLock;
    static LEVELS: OnceLock<BTreeMap<u32, Vec<CraftRank>>> = OnceLock::new();
    LEVELS.get_or_init(|| {
        let mut rows: BTreeMap<u32, Vec<CraftRank>> = BTreeMap::new();
        for line in include_str!("../content/crafting-levels.tsv").lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("pyrealium-") {
                continue;
            }
            let mut fields = line.split('\t');
            let skill = u32::from_str_radix(fields.next().unwrap(), 16).unwrap();
            let level = fields.next().unwrap().parse().unwrap();
            let curve = fields.next().unwrap().parse().unwrap();
            let learning_limit = fields.next().unwrap().parse().unwrap();
            let cxp = fields.next().unwrap().parse().unwrap();
            rows.entry(skill).or_default().push(CraftRank {
                level,
                curve,
                learning_limit,
                cxp,
            });
        }
        for ranks in rows.values() {
            assert!(ranks.iter().enumerate().all(|(index, rank)| {
                rank.level == (index + 1) as u16
                    && (index == 0 || rank.curve >= ranks[index - 1].curve)
            }));
        }
        assert_eq!(rows.values().map(Vec::len).sum::<usize>(), 1750);
        rows
    })
}

/// Rank reached by `crafting_xp` on one imported crafting skill.
pub fn rank(skill: u32, crafting_xp: u64) -> Option<CraftRank> {
    let ranks = crafting_levels().get(&skill)?;
    let index = ranks.partition_point(|rank| u64::from(rank.curve) <= crafting_xp);
    ranks.get(index.saturating_sub(1)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_crafting_curves_match_the_pyreal_dump() {
        let butchery = rank(0x6f002560, 0).unwrap();
        assert_eq!(
            butchery,
            CraftRank {
                level: 1,
                curve: 0,
                learning_limit: 120,
                cxp: 2
            }
        );
        let recipes = rank(CRAFT_SKILL_DID, 0).unwrap();
        assert_eq!(recipes.level, 1);
        assert_eq!(recipes.cxp, 0);
        assert_eq!(rank(CRAFT_SKILL_DID, 10).unwrap().level, 2);
        assert_eq!(rank(0x6f005276, 0).unwrap().level, 1);
        assert!(rank(1, 0).is_none());
    }
}
