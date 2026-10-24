//! Experimental empty-player HUD profile. Protocol layouts, not captured player state.
//! See docs/world-evidence.md for capture and matching-client evidence boundaries.
use crate::{script, world::WorldError};

pub const LOAD: u32 = 0x82120027;
pub fn death_notice(reason: &str) -> Result<Vec<u8>, WorldError> {
    script::private_call(0x8207002b, &script::literal(reason)?)
}
pub fn lock_player(locked: bool) -> Result<Vec<u8>, WorldError> {
    script::private_call(0x82070033, &script::integer(u32::from(locked)))
}
pub fn lifestone_bound(
    owner: u64,
    target: u64,
    point: crate::movement::Placement,
) -> Result<Vec<u8>, WorldError> {
    use_success(owner, target, point, None)
}
pub fn portal_used(
    owner: u64,
    target: u64,
    point: crate::movement::Placement,
    distance: f32,
) -> Result<Vec<u8>, WorldError> {
    if !distance.is_finite() || !(0.0..=4.0).contains(&distance) {
        return Err(WorldError::InvalidValue);
    }
    use_success(owner, target, point, Some(distance))
}
fn use_success(
    owner: u64,
    target: u64,
    point: crate::movement::Placement,
    portal_distance: Option<f32>,
) -> Result<Vec<u8>, WorldError> {
    if !point.valid() {
        return Err(WorldError::InvalidValue);
    }
    let mut fields = [0u32; 20];
    fields[0] = 2;
    for index in [1, 15] {
        fields[index] = u32::MAX;
    }
    fields[2] = target as u32;
    fields[3] = (target >> 32) as u32;
    fields[4] = 3;
    fields[5] = 0x1f;
    fields[6] = owner as u32;
    fields[7] = (owner >> 32) as u32;
    fields[8] = (-1f32).to_bits();
    fields[11] = 0x405;
    fields[17] = 0x13c;
    let mut tags = [
        1, 1, 0x84, 0xcd, 1, 4, 0x84, 0xcd, 3, 0x82, 0xcd, 2, 0x82, 0xcd, 2, 1, 2, 2, 2, 2,
    ];
    if let Some(distance) = portal_distance {
        fields[8] = distance.to_bits();
        fields[11] = 0;
        fields[12] = owner as u32;
        fields[13] = (owner >> 32) as u32;
        fields[17] = 0x3b4;
        fields[18] = 1;
        tags = [
            1, 1, 0x84, 0xcd, 1, 4, 0x84, 0xcd, 4, 0x82, 0xcd, 2, 0x84, 0xcd, 4, 1, 4, 2, 4, 4,
        ];
    }
    let root: Vec<_> = fields.into_iter().zip(tags).collect();
    let mut position = vec![point.cell];
    position.extend(
        point
            .coordinates
            .into_iter()
            .chain(point.orientation())
            .map(f32::to_bits),
    );
    let mut args = Vec::new();
    graph(
        &mut args,
        &[
            Object::interpreted(0x245, &root),
            Object::native(0x6a, &[0x4e1aec40, 0x25000014, 0]),
            Object::native(0x5a, &position),
        ],
    );
    script::private_call(0x82160021, &args)
}
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
    validate_layouts(
        bytes,
        &[
            (0x1a2, 3),
            (0x1f9, 6),
            (0x1f8, 51),
            (0x245, 20),
            (0x1f6, 19),
            (0x1f5, 7),
        ],
    )
}
pub fn validate_skills_wlib(bytes: &[u8]) -> Result<(), WorldError> {
    validate_layouts(bytes, &[(0x231, 15)])
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

pub fn hand_equipment(
    owner: u64,
    item: u64,
    slot: u16,
    other_hands: u16,
    metadata: crate::inventory_request::Equipment,
    equipped: bool,
) -> Result<Vec<u8>, WorldError> {
    if !matches!(slot, 0x40 | 0x80) || other_hands & !0xc0 != 0 || other_hands & slot != 0 {
        return Err(WorldError::InvalidValue);
    }
    let mut fields = [0; 19];
    fields[1] = 2;
    fields[2] = 1;
    fields[3] = item as u32;
    fields[4] = (item >> 32) as u32;
    fields[5] = u32::from(other_hands);
    fields[7] = u32::from(slot);
    for index in [8, 10] {
        fields[index] = owner as u32;
        fields[index + 1] = (owner >> 32) as u32;
    }
    fields[12] = if equipped {
        if slot == 0x40 { 0xce } else { 0x8e }
    } else {
        0x9e
    };
    fields[14] = u32::from(slot | other_hands);
    fields[15] = u32::from(slot);
    let mut tags = [
        2, 1, 2, 0x84, 0xcd, 4, 2, 4, 0x84, 0xcd, 0x84, 0xcd, 2, 2, 2, 4, 0x82, 0xcd, 2,
    ];
    if !equipped {
        tags[7] = 2;
        tags[15] = 2;
    }
    let root: Vec<_> = fields.into_iter().zip(tags).collect();
    let data = if equipped {
        vec![
            (0, 4),
            (metadata.primary, 4),
            (metadata.secondary, 4),
            (metadata.valid, 4),
            (0, 2),
            (metadata.preferred, 4),
            (0, 4),
        ]
    } else {
        vec![(0, 2); 7]
    };
    let mut arguments = Vec::new();
    graph(
        &mut arguments,
        &[
            Object::interpreted(0x1f6, &root),
            Object::interpreted(0x1f5, &data),
        ],
    );
    script::private_call(if equipped { 0x820e0051 } else { 0x820e0056 }, &arguments)
}

/// Deliberately empty development state: no grants, active effects, XP or gold.
/// This initializes presentation only. It is not an inventory or progression store.
pub fn empty_player() -> Result<Vec<u8>, WorldError> {
    player(None, None)
}
pub fn skill_credits(credits: u16) -> Result<Vec<u8>, WorldError> {
    let mut args = Vec::new();
    graph(
        &mut args,
        &[
            Object::interpreted(
                0x233,
                &[
                    (u32::from(credits), INT),
                    (0, 0x82),
                    (0, 0xcd),
                    (0, INT),
                    (2, REF),
                    (0, INT),
                    (3, REF),
                    (NULL, REF),
                ],
            ),
            Object::native(1, &[0]),
            Object::native(1, &[0]),
        ],
    );
    script::private_call(0x82140011, &args)
}
fn skill_object(id: u32, entry: &crate::skills::Trained) -> Object {
    Object::interpreted(
        0x231,
        &[
            (0, 0xc3),
            (0xbff00000, 0xcd),
            (u32::from(entry.rank), INT),
            (0, 0x83),
            (0, 0xcd),
            (entry.xp as u32, 0x82),
            ((entry.xp >> 32) as u32, 0xcd),
            (3, INT),
            (0, 0xc3),
            (0xbff00000, 0xcd),
            (0, INT),
            (0, INT),
            (1, INT),
            (0, INT),
            (id, 4),
        ],
    )
}
pub fn skill_update(id: u32, entry: &crate::skills::Trained) -> Result<Vec<u8>, WorldError> {
    let mut args = Vec::new();
    graph(&mut args, &[skill_object(id, entry)]);
    script::private_call(0x8214000d, &args)
}
pub fn skill_remove(id: u32) -> Result<Vec<u8>, WorldError> {
    script::private_call(0x82140014, &script::integer(id))
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
    player_all(settings, hotbar, inventory, progression, None)
}
pub fn player_all(
    settings: Option<&crate::preferences::UiSettings>,
    hotbar: Option<&crate::preferences::Hotbar>,
    inventory: Option<&crate::inventory::Inventory>,
    progression: Option<&crate::progression::Progression>,
    skills: Option<&crate::skills::Book>,
) -> Result<Vec<u8>, WorldError> {
    player_all_quests(settings, hotbar, inventory, progression, skills, None)
}
pub fn player_all_quests(
    settings: Option<&crate::preferences::UiSettings>,
    hotbar: Option<&crate::preferences::Hotbar>,
    inventory: Option<&crate::inventory::Inventory>,
    progression: Option<&crate::progression::Progression>,
    skills: Option<&crate::skills::Book>,
    quests: Option<&crate::quests::Book>,
) -> Result<Vec<u8>, WorldError> {
    let mut args = script::integer(progression.map_or(0, |state| state.gold));
    graph(
        &mut args,
        &[
            Object::interpreted(0x1a4, &[(0, INT), (2, REF)]),
            Object::native(0x11, &[0]), // Story chapter hash.
        ],
    );
    if let Some(book) = quests {
        args.extend(book.list_graph()?);
    } else {
        graph(&mut args, &[Object::native(0xa1, &[0])]);
    }
    // GameplayOptionsProfile::StreamPack 006000b0 starts with a u64 mask.
    // A zero mask retains native Clear/default values; no captured settings.
    let options = crate::preferences::options_words(settings, hotbar)?;
    graph(&mut args, &[Object::native(0x9b, &options)]);
    let mut trained = skills
        .into_iter()
        .flat_map(|book| book.entries())
        .collect::<Vec<_>>();
    trained.sort_unstable_by_key(|(id, _)| (id % 11, *id));
    let mut skill_map = vec![trained.len() as u32];
    for (index, (id, _)) in trained.iter().enumerate() {
        skill_map.extend([*id, 5 + index as u32]);
    }
    let mut skill_objects =
        vec![
            Object::interpreted(
                0x233,
                &[
                    (
                        progression.map_or(0, |state| {
                            u32::from(skills.map_or(state.level().credits, |book| {
                                book.available_credits(state)
                            }))
                        }),
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
            Object::native(0x11, &skill_map),
        ];
    for (id, entry) in trained {
        skill_objects.push(skill_object(id, entry));
    }
    graph(&mut args, &skill_objects);
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
