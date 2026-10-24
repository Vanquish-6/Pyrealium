//! Explicit development equipment grant, not recovered starter entitlement.
use crate::{
    dat::Dat,
    inventory::{Item, Location},
    inventory_request::Metadata,
};
use std::io;

// Immutable for journal event t1. A changed kit requires a new event revision.
pub const DEFINITION: u32 = 0x47000f1e;
const PHYSICAL: u32 = 0x4700035b;
const VISUAL: u32 = 0x1f000056;

pub fn item(id: u64, slot: u16) -> Item {
    Item {
        id,
        definition: DEFINITION,
        visual: VISUAL,
        quantity: 1,
        stack_limit: 1,
        equipment_mask: 0xc0,
        location: Location::Bag(slot),
    }
}

pub fn validate(portal: &mut Dat) -> io::Result<Metadata> {
    let invalid = || io::Error::other("training kit is incompatible with supplied DAT assets");
    portal.read(DEFINITION)?;
    portal.read(VISUAL)?;
    // The known weapon description has additional crafting/string fields that
    // the generic metadata reader deliberately does not accept. Verify the
    // complete known record before supplying its observed native properties.
    // Do not widen the generic reader or infer offsets for other definitions.
    fn fingerprint(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }
    let ui = portal.read(0x81000f1e)?;
    if ui.len() != 332 || fingerprint(&ui) != 0xf5e92f85457db882 {
        return Err(invalid());
    }
    let metadata = Metadata {
        icon: 0x2000012f,
        physical: PHYSICAL,
        equipment: Some(crate::inventory_request::Equipment {
            valid: 0xc0,
            preferred: 0x40,
            primary: 1,
            secondary: 0x40000006,
        }),
    };
    portal.read(metadata.icon)?;
    let hand = metadata.equipment.ok_or_else(invalid)?;
    if metadata.physical != PHYSICAL
        || hand.valid != 0xc0
        || hand.preferred != 0x40
        || hand.hand_slot(&item(crate::inventory::ITEM_NAMESPACE + 1, 0)) != Some(0x40)
    {
        return Err(invalid());
    }
    // Narrow native physical-definition prefix; no scan for a plausible DID.
    let physical = portal.read(PHYSICAL)?;
    if physical.len() != 36 || fingerprint(&physical) != 0xe3dc378c33dd44d1 {
        return Err(invalid());
    }
    let expected: Vec<u8> = [0x8bu32, PHYSICAL, 0x40000001, VISUAL]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    if physical.get(..16) != Some(expected.as_slice()) {
        return Err(invalid());
    }
    Ok(metadata)
}
