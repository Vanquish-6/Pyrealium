//! Experimental empty-player HUD profile. Protocol layouts, not captured player state.
//! See docs/world-evidence.md for capture and matching-client evidence boundaries.
use crate::{script, world::WorldError};

pub const LOAD: u32 = 0x82120027;
const REF: u8 = 1;
const INT: u8 = 2;
const NULL: u32 = u32::MAX;

/// Check serialized interpreted-object sizes before enabling this fixed profile.
/// VTableSection::StreamPack 006e89d0 stores two nested word arrays, followed by
/// (size, checksum) package-info pairs. This checks layout, not script semantics.
pub fn validate_wlib(bytes: &[u8]) -> Result<(), WorldError> {
    validate_layouts(bytes, &[(0x1a4, 2), (0x233, 8), (0x14f, 14), (0x24a7, 2)])
}
pub fn validate_inventory_wlib(bytes: &[u8]) -> Result<(), WorldError> {
    validate_layouts(bytes, &[(0x1a2, 3), (0x1f9, 6)])
}
fn validate_layouts(bytes: &[u8], layouts: &[(usize, u32)]) -> Result<(), WorldError> {
    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }
    impl Reader<'_> {
        fn take(&mut self, size: usize) -> Result<&[u8], WorldError> {
            let end = self
                .pos
                .checked_add(size)
                .ok_or(WorldError::LimitExceeded)?;
            let value = self.bytes.get(self.pos..end).ok_or(WorldError::Truncated)?;
            self.pos = end;
            Ok(value)
        }
        fn word(&mut self) -> Result<u32, WorldError> {
            Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
        }
        fn nested(&mut self) -> Result<(), WorldError> {
            let count = self.word()?;
            if count > 32768 {
                return Err(WorldError::LimitExceeded);
            }
            for _ in 0..count {
                let count = self.word()? as usize;
                self.take(count.checked_mul(4).ok_or(WorldError::LimitExceeded)?)?;
            }
            Ok(())
        }
    }
    let mut reader = Reader { bytes, pos: 0 };
    if reader.word()? != 0x56000005 || reader.word()? != 0x1a015342 || reader.word()? != 15 {
        return Err(WorldError::InvalidValue);
    }
    let mut vtable = None;
    let mut sections = 0;
    while reader.pos < bytes.len() {
        sections += 1;
        if sections > 128 {
            return Err(WorldError::LimitExceeded);
        }
        let kind = reader.word()?;
        let size = reader.word()? as usize;
        let start = reader.pos;
        reader.take(size)?;
        if kind == 6 {
            if vtable.is_some() {
                return Err(WorldError::InvalidValue);
            }
            vtable = Some(&bytes[start..start + size]);
        }
    }
    let mut reader = Reader {
        bytes: vtable.ok_or(WorldError::InvalidValue)?,
        pos: 0,
    };
    reader.nested()?;
    reader.nested()?;
    let count = reader.word()? as usize;
    if count > 32768 {
        return Err(WorldError::LimitExceeded);
    }
    let info = reader.take(count.checked_mul(8).ok_or(WorldError::LimitExceeded)?)?;
    for &(package, size) in layouts {
        let entry = info
            .get(package * 8..package * 8 + 4)
            .ok_or(WorldError::InvalidValue)?;
        if entry != size.to_le_bytes() {
            return Err(WorldError::InvalidValue);
        }
    }
    Ok(())
}

struct Object {
    kind: u32,
    native: Vec<u32>,
    fields: Vec<(u32, u8)>,
}
impl Object {
    fn native(kind: u16, payload: &[u32]) -> Self {
        Self {
            kind: 0xffff0000 | u32::from(kind),
            native: payload.to_vec(),
            fields: Vec::new(),
        }
    }
    fn interpreted(package: u16, fields: &[(u32, u8)]) -> Self {
        Self {
            kind: u32::from(package) << 16,
            native: Vec::new(),
            fields: fields.to_vec(),
        }
    }
}
fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}
fn graph(bytes: &mut Vec<u8>, objects: &[Object]) {
    word(bytes, 5); // PackBuffer package/reference graph.
    word(bytes, objects.len() as u32);
    for id in 1..=objects.len() {
        word(bytes, id as u32);
    }
    for object in objects {
        word(bytes, 0x44000100); // Serialized reference flags.
        word(bytes, 0); // No external package-state dependency.
        word(bytes, object.kind);
        for value in &object.native {
            word(bytes, *value);
        }
        word(bytes, object.fields.len() as u32);
        for (value, _) in &object.fields {
            word(bytes, *value);
        }
        for (_, tag) in &object.fields {
            bytes.push(*tag);
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
}

/// Deliberately empty development state: no grants, active effects, XP or gold.
/// This initializes presentation only. It is not an inventory or progression store.
pub fn empty_player() -> Result<Vec<u8>, WorldError> {
    player(None, None)
}
pub fn player(
    settings: Option<&crate::preferences::UiSettings>,
    hotbar: Option<&crate::preferences::Hotbar>,
) -> Result<Vec<u8>, WorldError> {
    player_inventory(settings, hotbar, None)
}
pub fn player_inventory(
    settings: Option<&crate::preferences::UiSettings>,
    hotbar: Option<&crate::preferences::Hotbar>,
    inventory: Option<&crate::inventory::Inventory>,
) -> Result<Vec<u8>, WorldError> {
    player_state(settings, hotbar, inventory, None)
}
pub fn player_state(
    settings: Option<&crate::preferences::UiSettings>,
    hotbar: Option<&crate::preferences::Hotbar>,
    inventory: Option<&crate::inventory::Inventory>,
    progression: Option<&crate::progression::Progression>,
) -> Result<Vec<u8>, WorldError> {
    let mut args = script::integer(progression.map_or(0, |state| state.gold));
    graph(
        &mut args,
        &[
            Object::interpreted(0x1a4, &[(0, INT), (2, REF)]),
            Object::native(0x11, &[0]), // Story chapter hash.
        ],
    );
    graph(&mut args, &[Object::native(0xa1, &[0])]); // gmQuestInfoList.
    // GameplayOptionsProfile::StreamPack 006000b0 starts with a u64 mask.
    // A zero mask retains native Clear/default values; no captured settings.
    let options = crate::preferences::options_words(settings, hotbar)?;
    graph(&mut args, &[Object::native(0x9b, &options)]);
    graph(
        &mut args,
        &[
            Object::interpreted(
                0x233,
                &[
                    (
                        progression.map_or(0, |state| u32::from(state.level().credits)),
                        INT,
                    ),
                    (0, 0x82),
                    (0, 0xcd),
                    (0, INT),
                    (2, REF),
                    (0, INT),
                    (3, REF),
                    (4, REF),
                ],
            ),
            Object::native(1, &[0]),
            Object::native(1, &[0]),
            Object::native(0x11, &[0]), // No trained skill entries.
        ],
    );
    graph(
        &mut args,
        &[
            Object::interpreted(
                0x14f,
                &[
                    (NULL, REF),
                    (2, REF),
                    (NULL, REF),
                    (0, INT),
                    (NULL, REF),
                    (0, 0xc3),
                    (0xbff00000, 0xcd), // Native -1.0 double sentinel.
                    (NULL, REF),
                    (NULL, REF),
                    (0, INT),
                    (NULL, REF),
                    (NULL, REF),
                    (NULL, REF),
                    (3, REF),
                ],
            ),
            Object::native(1, &[0]),
            Object::native(1, &[0]),
        ],
    );
    args.extend(script::integer(0)); // Empty effect/inventory sequence.
    inventory_map(&mut args, inventory, true);
    inventory_map(&mut args, inventory, false);
    // Four empty development bags. Capacity is policy, not a recovered grant.
    graph(
        &mut args,
        &[
            Object::native(0x65, &[4, 2, 3, 4, 5]),
            Object::interpreted(0x24a7, &[(42, INT), (bag_count(inventory, 0, 42), INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (bag_count(inventory, 42, 54), INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (bag_count(inventory, 54, 66), INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (bag_count(inventory, 66, 78), INT)]),
        ],
    );
    graph(&mut args, &[Object::native(0x3b, &[0])]);
    let mut bag_items = Vec::new();
    if let Some(inventory) = inventory {
        for item in inventory.items() {
            if let crate::inventory::Location::Bag(slot) = item.location {
                bag_items.resize(bag_items.len().max(slot as usize + 1), 0u64);
                bag_items[slot as usize] = item.id;
            }
        }
    }
    let mut list = vec![bag_items.len() as u32];
    for id in bag_items {
        list.extend([id as u32, (id >> 32) as u32]);
    }
    graph(&mut args, &[Object::native(0x3b, &list)]);
    args.extend(script::integer(1));
    args.extend(script::integer(0));
    script::private_call(LOAD, &args)
}

fn bag_count(inventory: Option<&crate::inventory::Inventory>, start: u16, end: u16) -> u32 {
    inventory.map(|inventory|inventory.items().filter(|item|matches!(item.location,crate::inventory::Location::Bag(slot) if slot>=start && slot<end)).count() as u32).unwrap_or(0)
}
fn inventory_map(
    out: &mut Vec<u8>,
    inventory: Option<&crate::inventory::Inventory>,
    equipment: bool,
) {
    use crate::inventory::Location;
    let mut items: Vec<_> = inventory
        .into_iter()
        .flat_map(|inventory| inventory.items())
        .filter(|item| !equipment || matches!(item.location, Location::Equipped(_)))
        .collect();
    // LRHash uses the full u64 remainder; the equipment ARHash uses u32 slots.
    items.sort_unstable_by_key(|item| {
        let key = if equipment {
            match item.location {
                Location::Equipped(slot) => u64::from(slot),
                _ => 0,
            }
        } else {
            item.id
        };
        (key % 11, key)
    });
    let mut payload = vec![items.len() as u32];
    for (index, item) in items.iter().enumerate() {
        if equipment {
            let Location::Equipped(slot) = item.location else {
                unreachable!()
            };
            payload.push(u32::from(slot));
        } else {
            payload.extend([item.id as u32, (item.id >> 32) as u32]);
        }
        payload.push(2 + index as u32 * 5);
    }
    let mut objects = vec![Object::native(
        if equipment { 0x11 } else { 0x40 },
        &payload,
    )];
    for item in items {
        let root = objects.len() as u32 + 1;
        let slot = match item.location {
            Location::Equipped(slot) => u32::from(slot),
            _ => 0,
        };
        objects.push(Object::interpreted(
            0x1f9,
            &[
                (root + 1, REF),
                (u32::from(item.equipment_mask), INT),
                (slot, INT),
                (0, INT),
                (item.id as u32, 0x82),
                ((item.id >> 32) as u32, 0xcd),
            ],
        ));
        objects.push(Object::interpreted(
            0x1a2,
            &[(root + 2, REF), (root + 3, REF), (root + 4, REF)],
        ));
        objects.push(Object::native(0x70, &[1f32.to_bits(); 3]));
        objects.push(Object::native(0xf, &[0])); // No unverified appearance overrides.
        objects.push(Object::native(0x71, &[2, item.visual]));
    }
    graph(out, &objects);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_gate_rejects_changed_sizes_truncation_and_unbounded_counts() {
        let mut section = Vec::new();
        for value in [1, 0, 1, 0, 0x24a8] {
            word(&mut section, value);
        }
        section.resize(section.len() + 0x24a8 * 8, 0);
        for (package, size) in [(0x1a4, 2u32), (0x233, 8), (0x14f, 14), (0x24a7, 2)] {
            section[20 + package * 8..24 + package * 8].copy_from_slice(&size.to_le_bytes());
        }
        let mut wlib = Vec::new();
        for value in [0x56000005, 0x1a015342, 15, 6, section.len() as u32] {
            word(&mut wlib, value);
        }
        wlib.extend(&section);
        validate_wlib(&wlib).unwrap();
        for length in [0, 1, 4, 8, 11, 12, 16, 19, 20, 24, wlib.len() - 1] {
            assert!(validate_wlib(&wlib[..length]).is_err());
        }
        for (offset, value) in [
            (8, 16),
            (16, u32::MAX),
            (20, 32769),
            (28, 32769),
            (36, u32::MAX),
            (40 + 0x14f * 8, 13),
        ] {
            let mut invalid = wlib.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(validate_wlib(&invalid).is_err());
        }
        let mut duplicate = wlib.clone();
        duplicate.extend(&wlib[12..]);
        assert!(validate_wlib(&duplicate).is_err());
    }
    #[test]
    fn profile_has_fourteen_arguments_and_closed_reference_graphs() {
        let bytes = empty_player().unwrap();
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(&words[..2], &[0x10066, LOAD]);
        assert_eq!(words[2] as usize, bytes.len() - 12);
        let mut p = 3;
        let mut arguments = 0;
        let mut packages = Vec::new();
        while p < words.len() {
            arguments += 1;
            let tag = words[p];
            p += 1;
            if tag == 1 {
                p += 1;
                continue;
            }
            assert_eq!(tag, 5);
            let count = words[p];
            p += 1;
            assert!((1..=5).contains(&count));
            assert_eq!(
                &words[p..p + count as usize],
                &(1..=count).collect::<Vec<_>>()
            );
            p += count as usize;
            for _ in 0..count {
                assert_eq!(words[p], 0x44000100);
                assert_eq!(words[p + 1], 0);
                let kind = words[p + 2];
                p += 3;
                packages.push(kind);
                if kind & 0xffff != 0 {
                    if kind & 0xffff == 0x9b {
                        assert_eq!(&words[p..p + 2], &[0, 0]);
                        p += 2;
                    } else {
                        let n = words[p];
                        p += 1;
                        let stride = if kind & 0xffff == 0x65 {
                            1
                        } else {
                            assert_eq!(n, 0);
                            0
                        };
                        for reference in &words[p..p + n as usize * stride] {
                            assert!((1..=count).contains(reference));
                        }
                        p += n as usize * stride;
                    }
                }
                let n = words[p] as usize;
                p += 1;
                let values = &words[p..p + n];
                p += n;
                let tags = &bytes[p * 4..p * 4 + n];
                for (&value, &tag) in values.iter().zip(tags) {
                    if tag == REF {
                        assert!(value == NULL || (1..=count).contains(&value));
                    }
                }
                p += n.div_ceil(4);
            }
        }
        assert_eq!(arguments, 14);
        assert_eq!(p, words.len());
        assert!(packages.contains(&0x02330000));
        assert!(bytes.len() < 2048);
    }
}
