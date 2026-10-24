//! Native single-item acquisition/removal messages for authored gameplay grants.
//!
//! Acquisition profile: retained picking.up.item.elog, private 820e005e,
//! interpreted package 1f8 with 51 fields. Removal profile: selling.items.in.
//! inventory.elog and crafting.item.2.elog, private 820e0058 with a tagged IID.
//! Native 7148 WLib resolves these to handlers 1aa400e5 and 1aa60002; heap wire
//! layout follows HeapObject::Pack 006dbb00. Capture build identity is unknown.
//! This does not implement ground-item authority, corpse UI or original loot.

use crate::{
    dat::Dat,
    inventory::{BAG_SLOTS, Item, Location},
    inventory_request::Metadata,
    world::WorldError,
};
use std::io;

pub const ACQUIRE: u32 = 0x820e005e;
pub const REMOVE: u32 = 0x820e0058;
pub const POTION_DEFINITION: u32 = 0x47001745;
pub const POTION_VISUAL: u32 = 0x1f000e3f;
pub const POTION_PHYSICAL: u32 = 0x47001773;
pub const POTION_ICON: u32 = 0x20000268;
/// Authored development balance, not inferred from the item's existence.
pub const POTION_HEALTH: u32 = 25;
pub const POTION_COOLDOWN_MS: u64 = 5000;

const ACQUIRE_TAGS: [u8; 51] = [
    2, 2, 0x84, 0xcd, 2, 2, 4, 2, 0x84, 0xcd, 2, 2, 2, 2, 2, 2, 0x84, 0xcd, 0x82, 0xcd, 2, 2, 4, 2,
    2, 2, 4, 2, 0x84, 0xcd, 1, 2, 0x84, 0xcd, 2, 2, 0x84, 0xcd, 4, 2, 2, 2, 2, 2, 0x82, 0xcd, 2, 2,
    2, 2, 2,
];

pub fn potion(id: u64, slot: u16) -> Item {
    Item {
        id,
        definition: POTION_DEFINITION,
        visual: POTION_VISUAL,
        quantity: 1,
        stack_limit: 1,
        equipment_mask: 0,
        location: Location::Bag(slot),
    }
}

/// Validate server-owned definition/icon/physical metadata before enabling the
/// authored potion profile. No item-use request supplies any of these assets.
pub fn validate_potion(portal: &mut Dat) -> io::Result<Metadata> {
    let invalid =
        || io::Error::other("development potion is incompatible with supplied DAT assets");
    portal.read(POTION_DEFINITION)?;
    portal.read(POTION_VISUAL)?;
    // The generic qualities reader deliberately rejects this record's string
    // quality tail. Accept only the complete known descriptor, rather than
    // relaxing that reader or scanning for plausible asset IDs.
    let descriptor = portal.read(0x81001745)?;
    let fingerprint = descriptor.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    if descriptor.len() != 288 || fingerprint != 0xeffeb5ec9e8b026a {
        return Err(invalid());
    }
    portal.read(POTION_ICON)?;
    let metadata = Metadata {
        icon: POTION_ICON,
        physical: POTION_PHYSICAL,
        equipment: None,
    };
    let physical = portal.read(POTION_PHYSICAL)?;
    let expected: Vec<u8> = [0x0bu32, POTION_PHYSICAL, 0x40000001, POTION_VISUAL]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    if physical.as_slice() != expected.as_slice() {
        return Err(invalid());
    }
    Ok(metadata)
}

/// Announce one newly owned item in an authoritative final bag slot. The server
/// creates the contained object first, then queues this message. Native source
/// owner fields remain null: this profile does not claim a prior owned item.
pub fn acquired(owner: u64, item: &Item, metadata: Metadata) -> Result<Vec<u8>, WorldError> {
    let Location::Bag(slot) = item.location else {
        return Err(WorldError::InvalidValue);
    };
    if [0, u64::MAX].contains(&owner)
        || [0, u64::MAX].contains(&item.id)
        || owner == item.id
        || ((item.quantity != 1 || item.stack_limit != 1)
            && !(item.definition == crate::crafting::TALLOW
                && item.visual == 0x1f000234 && item.equipment_mask == 0
                && item.quantity == crate::crafting::TALLOW_QUANTITY && item.stack_limit == crate::crafting::TALLOW_QUANTITY))
        || slot >= BAG_SLOTS
        || item.definition >> 24 != 0x47
        || item.visual >> 24 != 0x1f
        // Package 1f8 field 6 is the optional AppearanceFile DID, not the
        // descriptor's texture icon. Tallow's pinned profile has no appearance
        // animation; preserve its null DID rather than inventing an animation.
        // The nested inventory/picking.up.item.elog retains this exact full
        // acquisition shape with field 6 zero and DID tag 4 (item 470017f3).
        || (metadata.icon >> 24 != 0x20
            && !(item.definition == crate::crafting::CATALYST
                && item.visual == 0x1f000234
                && metadata.physical == 0x47000260
                && metadata.icon == 0))
        || metadata.physical >> 24 != 0x47
    {
        return Err(WorldError::InvalidValue);
    }
    let mut fields = [0u32; 51];
    for index in [2, 8, 32] {
        fields[index] = owner as u32;
        fields[index + 1] = (owner >> 32) as u32;
    }
    for index in [28, 44] {
        fields[index] = item.id as u32;
        fields[index + 1] = (item.id >> 32) as u32;
    }
    for index in [10, 23, 26, 35, 41, 47, 49] {
        fields[index] = 1;
    }
    fields[6] = metadata.icon;
    fields[12] = 0x0c;
    fields[22] = item.definition;
    fields[24] = u32::from(slot);
    fields[30] = u32::MAX;
    fields[38] = metadata.physical;
    fields[50] = u32::from(slot);
    if item.definition == crate::crafting::TALLOW && item.quantity > 1 {
        // Captured stacked Tallow pickups retain both moved and final counts.
        fields[26] = u32::from(item.quantity);
        fields[48] = u32::from(item.quantity);
    }
    let mut tags = ACQUIRE_TAGS;
    // Retained pickup captures exercise slot zero only (tag 2). Nonzero bag
    // indices use the same package's uint fields as native move acknowledgments;
    // matching-client nonzero-slot acquisition acceptance remains unverified.
    if slot != 0 {
        tags[24] = 4;
        tags[50] = 4;
    }
    if fields[48] != 0 {
        tags[48] = 4;
    }
    let mut args = Vec::with_capacity(284);
    for value in [5u32, 1, 1, 0x44000100, 0, 0x01f80000, 51] {
        args.extend(value.to_le_bytes());
    }
    for value in fields {
        args.extend(value.to_le_bytes());
    }
    args.extend(tags);
    args.push(0); // aligned interpreted field tags
    crate::script::private_call(ACQUIRE, &args)
}

/// Remove an item from the native inventory UI, without inventing a world drop.
/// Captured sales/crafting send this before the contained object's DestroyObject.
/// The server must remove durable ownership before queuing either message.
pub fn removed(item: u64) -> Result<Vec<u8>, WorldError> {
    if [0, u64::MAX].contains(&item) {
        return Err(WorldError::InvalidValue);
    }
    crate::script::private_call(REMOVE, &crate::script::identity(item))
}
