//! Frozen racial starter inventories, grounded in creation/login captures and
//! matching DAT definitions. Never infer fresh grants from item names alone.
//! Historical t1/t2 training grants remain immutable; this profile uses t4.
use crate::{
    dat::Dat,
    inventory::{Command, Inventory, Item, Location},
    inventory_request::Metadata,
    world::CharacterCreationRequest,
};
use std::{collections::BTreeMap, io};

struct Profile {
    race: u32,
    definition: u32,
    visual: u32,
    mask: u16,
    worn: u16,
    qualities_hash: u64,
    physical_hash: u64,
}
include!("starter_gear_data.rs");

fn invalid() -> io::Error {
    io::Error::other("unsupported or incompatible starter gear profile")
}
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
pub fn is_apparel(definition: u32) -> bool {
    PROFILES
        .iter()
        .any(|profile| profile.definition == definition && profile.worn != 0)
}
pub fn is_weapon(definition: u32) -> bool {
    PROFILES
        .iter()
        .any(|profile| profile.definition == definition && profile.worn == 0)
}
pub fn is_sword(definition: u32) -> bool {
    matches!(definition, 0x470007ee | 0x470007f5 | 0x47000809)
}
pub(crate) fn hand_scale(definition: u32, visual: u32) -> Option<f32> {
    if definition == crate::training_kit::DEFINITION && visual == 0x1f000056 { return Some(1.0); }
    PROFILES.iter().find(|profile| profile.definition == definition && profile.visual == visual && profile.worn == 0)
        .map(|profile| if profile.definition == 0x470007ee { 0.8 } else { 1.0 })
}
pub(crate) fn quality_matches(definition: u32, bytes: &[u8]) -> bool {
    PROFILES.iter().any(|profile| profile.definition == definition && fingerprint(bytes) == profile.qualities_hash)
}
pub(crate) fn physical_matches(definition: u32, bytes: &[u8]) -> bool {
    PROFILES.iter().any(|profile| profile.definition == definition && fingerprint(bytes) == profile.physical_hash)
}
pub fn validate(portal: &mut Dat) -> io::Result<BTreeMap<u32, Metadata>> {
    let mut metadata = BTreeMap::new();
    for profile in PROFILES {
        portal.read(profile.definition)?;
        portal.read(profile.visual)?;
        if fingerprint(&portal.read(0x81000000 | (profile.definition & 0xffffff))?)
            != profile.qualities_hash
        {
            return Err(invalid());
        }
        let info = Metadata::load(portal, profile.definition)?;
        if fingerprint(&portal.read(info.physical)?) != profile.physical_hash {
            return Err(invalid());
        }
        metadata.insert(profile.definition, info);
    }
    Ok(metadata)
}

/// Candidate for an immutable once-only grant. Existing items and occupied
/// equipment slots are retained. Already-owned starter definitions are not
/// duplicated, and an occupied clothing slot leaves the new item in the bag.
pub fn candidate(
    request: &CharacterCreationRequest,
    inventory: &Inventory,
    first: u64,
) -> io::Result<Inventory> {
    if !matches!(request.race, 1 | 2 | 4)
        || !matches!(request.sex, 0x1000 | 0x2000)
        || !inventory.dense_bags()
    {
        return Err(invalid());
    }
    let mut next = inventory.clone();
    let mut id = first;
    // Worn pieces need only a temporary bag slot, so process them before the
    // three permanent bag weapons. A full final kit needs no spare fourth slot.
    let apparel = PROFILES
        .iter()
        .filter(|profile| profile.race == request.race && profile.worn != 0);
    let weapons = PROFILES
        .iter()
        .filter(|profile| profile.race == request.race && profile.worn == 0);
    for profile in apparel.chain(weapons) {
        if next
            .items()
            .any(|item| item.definition == profile.definition)
        {
            continue;
        }
        let slot = next
            .free_slot()
            .ok_or_else(|| io::Error::other("Make room for your racial starter gear."))?;
        let item = Item {
            id,
            definition: profile.definition,
            visual: profile.visual,
            quantity: 1,
            stack_limit: 1,
            equipment_mask: profile.mask,
            location: Location::Bag(slot),
        };
        next = next
            .apply(&Command::Grant(item))
            .map_err(io::Error::other)?;
        if profile.worn != 0
            && !next
                .items()
                .any(|item| item.location == Location::Equipped(profile.worn))
        {
            next = next
                .apply(&Command::Wear {
                    item: id,
                    slot: profile.worn,
                })
                .map_err(io::Error::other)?;
        }
        id = id.checked_add(1).ok_or_else(invalid)?;
    }
    Ok(next)
}
