//! Experimental empty-player HUD profile. Protocol layouts, not captured player state.
//! See docs/world-evidence.md for capture and matching-client evidence boundaries.
use crate::{script, world::WorldError};

pub const LOAD: u32 = 0x82120027;
const REF: u8 = 1;
const INT: u8 = 2;
const NULL: u32 = u32::MAX;

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
    let mut args = script::integer(0); // Gold.
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
    graph(&mut args, &[Object::native(0x9b, &[0, 0])]);
    graph(
        &mut args,
        &[
            Object::interpreted(
                0x233,
                &[
                    (0, INT),
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
    graph(&mut args, &[Object::native(0x11, &[0])]); // Equipment slot map.
    graph(&mut args, &[Object::native(0x40, &[0])]); // Item instance map.
    // Four empty development bags. Capacity is policy, not a recovered grant.
    graph(
        &mut args,
        &[
            Object::native(0x65, &[4, 2, 3, 4, 5]),
            Object::interpreted(0x24a7, &[(42, INT), (0, INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (0, INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (0, INT)]),
            Object::interpreted(0x24a7, &[(12, INT), (0, INT)]),
        ],
    );
    graph(&mut args, &[Object::native(0x3b, &[0])]);
    graph(&mut args, &[Object::native(0x3b, &[0])]);
    args.extend(script::integer(1));
    args.extend(script::integer(0));
    script::private_call(LOAD, &args)
}

#[cfg(test)]
mod tests {
    use super::*;
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
