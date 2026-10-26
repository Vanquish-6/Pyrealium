//! Character bodies and creation appearance.
//!
//! Tables follow the 2012 Pyreal server's AccountManager (PhysiqueTypes from
//! enum 2300000A, per race/sex aspect and starter-clothing records, start
//! points and racial starter skills). Its conversion matches a retail 2017
//! human-female creation: the CharGenVerification VisualDesc carried flags
//! 0x122, parent 1f000024, scale (Build, Build, Height), part group 0x21 /
//! 40000002 and aspect 2000000F {1: SkinTone}, 20000010 {3: HeadDetail,
//! 0B: FrillColor}, 20000011 {4100001C: FaceDetail} and clothing 20000013 /
//! 20000017 / 20000019 {2: colour, 10: 1.0}, omitting zero-valued traits.
use crate::{movement::Placement, world::WorldError};

pub const HUMAN: u32 = 1;
pub const LUGIAN: u32 = 2;
pub const TUMEROK: u32 = 4;
pub const MALE: u32 = 0x1000;
pub const FEMALE: u32 = 0x2000;

// Physique keys (enum 2300000A).
const BUILD: u32 = 0x1;
const HEIGHT: u32 = 0x2;
const SKIN_TONE: u32 = 0x4;
const SKIN_DETAIL: u32 = 0x8;
const HEAD_DETAIL: u32 = 0x10;
const HEAD_FRILL: u32 = 0x20;
const FRILL_COLOR: u32 = 0x40;
const SPECIAL: u32 = 0x80;
const SHIRT: u32 = 0x100;
const PANTS: u32 = 0x200;
const BOOTS: u32 = 0x400;
const FACE_DETAIL: u32 = 0x800;
const EYE_COLOR: u32 = 0x1000;
const PHYSIQUES: [u32; 13] = [
    BUILD, HEIGHT, SKIN_TONE, SKIN_DETAIL, HEAD_DETAIL, HEAD_FRILL, FRILL_COLOR, SPECIAL, SHIRT,
    PANTS, BOOTS, FACE_DETAIL, EYE_COLOR,
];

/// Trait id each physique writes on its aspect.
fn trait_id(physique: u32) -> Option<u32> {
    Some(match physique {
        SKIN_TONE => 0x1,
        HEAD_DETAIL => 0x3,
        FRILL_COLOR => 0xb,
        HEAD_FRILL => 0xc,
        SKIN_DETAIL => 0xf,
        SPECIAL => 0x4100_0003,
        FACE_DETAIL => 0x4100_001c,
        EYE_COLOR => 0x4100_002e,
        _ => return None,
    })
}
const CLOTHING_COLOR: u32 = 0x2;
const CLOTHING_WORN: u32 = 0x10;

pub struct Body {
    pub race: u32,
    pub sex: u32,
    /// Parent VisualDesc.
    pub visual: u32,
    /// Race/sex entity carried by PlayerDesc.
    pub entity: u32,
    pub start: Placement,
    /// physique -> aspect record.
    aspects: &'static [(u32, u32)],
    /// physique colour -> starter clothing record.
    clothing: &'static [(u32, u32)],
    /// Racial basic attacks: level 1, no credits, cannot be untrained.
    pub starters: [u32; 3],
}

const TUMEROK_START: Placement = Placement {
    cell: 0x03fd0113,
    coordinates: [-4.16552, -27.519, -24.2035],
    heading: 236,
};
// The Lugian starter dungeon (lugian_start_position, cells 04xx0144) is absent
// from the supported cell_1.dat; the Linvak Tukal recall point is used instead,
// as the 2012 server did.
const LUGIAN_START: Placement = Placement {
    cell: 0x58740039,
    coordinates: [147.74, 15.13, 161.75],
    heading: 96,
};

const BODIES: [Body; 6] = [
    Body {
        race: HUMAN,
        sex: MALE,
        visual: 0x1f000023,
        entity: 0x470000cd,
        start: Placement::HUMAN_START,
        aspects: &[
            (SKIN_TONE, 0x2000000c),
            (FRILL_COLOR, 0x2000000d),
            (HEAD_DETAIL, 0x2000000d),
            (HEAD_FRILL, 0x2000000d),
            (SKIN_DETAIL, 0x2000000e),
            (FACE_DETAIL, 0x2000000e),
        ],
        clothing: &[(PANTS, 0x20000014), (SHIRT, 0x20000016), (BOOTS, 0x20000019)],
        starters: [0xb0, 0xcf, 0x60],
    },
    Body {
        race: HUMAN,
        sex: FEMALE,
        visual: 0x1f000024,
        entity: 0x470000ce,
        start: Placement::HUMAN_START,
        aspects: &[
            (SKIN_TONE, 0x2000000f),
            (FRILL_COLOR, 0x20000010),
            (HEAD_DETAIL, 0x20000010),
            (HEAD_FRILL, 0x20000010),
            (SKIN_DETAIL, 0x20000011),
            (FACE_DETAIL, 0x20000011),
        ],
        clothing: &[(PANTS, 0x20000013), (SHIRT, 0x20000017), (BOOTS, 0x20000019)],
        starters: [0xb0, 0xcf, 0x60],
    },
    Body {
        race: LUGIAN,
        sex: MALE,
        visual: 0x1f000025,
        entity: 0x470000cf,
        start: LUGIAN_START,
        aspects: &[
            (SKIN_TONE, 0x20000000),
            (FRILL_COLOR, 0x20000001),
            (HEAD_DETAIL, 0x20000001),
            (HEAD_FRILL, 0x20000001),
            (SKIN_DETAIL, 0x20000002),
            (FACE_DETAIL, 0x20000002),
        ],
        clothing: &[(BOOTS, 0x2000001c), (SHIRT, 0x200001f6), (PANTS, 0x200001f7)],
        starters: [0x166, 0x1a0, 0x111],
    },
    Body {
        race: LUGIAN,
        sex: FEMALE,
        visual: 0x1f000079,
        entity: 0x470000d1,
        start: LUGIAN_START,
        aspects: &[
            (SKIN_TONE, 0x20000003),
            (FRILL_COLOR, 0x20000004),
            (HEAD_DETAIL, 0x20000004),
            (HEAD_FRILL, 0x20000004),
            (SKIN_DETAIL, 0x20000005),
            (FACE_DETAIL, 0x20000005),
        ],
        clothing: &[(BOOTS, 0x20000043), (SHIRT, 0x200001f8), (PANTS, 0x200001f9)],
        starters: [0x166, 0x1a0, 0x111],
    },
    Body {
        race: TUMEROK,
        sex: MALE,
        visual: 0x1f000026,
        entity: 0x470000d0,
        start: TUMEROK_START,
        aspects: &[
            (SKIN_TONE, 0x20000006),
            (FRILL_COLOR, 0x20000007),
            (HEAD_DETAIL, 0x20000007),
            (HEAD_FRILL, 0x20000007),
            (SKIN_DETAIL, 0x20000008),
            (FACE_DETAIL, 0x20000008),
        ],
        clothing: &[(PANTS, 0x20000039), (SHIRT, 0x20000038)],
        starters: [0x2cc, 0x316, 0x2a0],
    },
    Body {
        race: TUMEROK,
        sex: FEMALE,
        visual: 0x1f00007a,
        entity: 0x470000d2,
        start: TUMEROK_START,
        aspects: &[
            (SKIN_TONE, 0x20000009),
            (FRILL_COLOR, 0x2000000a),
            (HEAD_DETAIL, 0x2000000a),
            (HEAD_FRILL, 0x2000000a),
            (SKIN_DETAIL, 0x2000000b),
            (FACE_DETAIL, 0x2000000b),
            (SPECIAL, 0x2000000a),
        ],
        // Retail 2017 fresh Tumerok females wear the male jerkin and leggings.
        clothing: &[(PANTS, 0x20000039), (SHIRT, 0x20000038)],
        starters: [0x2cc, 0x316, 0x2a0],
    },
];

pub fn body(race: u32, sex: u32) -> Option<&'static Body> {
    BODIES.iter().find(|body| body.race == race && body.sex == sex)
}

pub fn bodies() -> impl Iterator<Item = &'static Body> {
    BODIES.iter()
}

pub fn is_player_visual(visual: u32) -> bool {
    BODIES.iter().any(|body| body.visual == visual)
}

impl Body {
    /// Every DAT record this body's appearance references.
    pub fn records(&self) -> impl Iterator<Item = u32> + '_ {
        [self.visual, self.entity]
            .into_iter()
            .chain(self.aspects.iter().map(|&(_, aspect)| aspect))
            .chain(self.clothing.iter().map(|&(_, aspect)| aspect))
    }

    /// Full VisualDesc for this body with the creation choices applied.
    /// Rejects unknown physique keys and out-of-range values.
    pub fn visual_desc(&self, physical: &[(u32, u32)]) -> Result<Vec<u8>, WorldError> {
        let mut values = std::collections::BTreeMap::new();
        for &(key, bits) in physical {
            let value = f32::from_bits(bits);
            let (low, high) = if matches!(key, BUILD | HEIGHT) { (0.5, 1.5) } else { (0.0, 1.0) };
            if !PHYSIQUES.contains(&key) || !value.is_finite() || value < low || value > high {
                return Err(WorldError::InvalidValue);
            }
            values.insert(key, value);
        }
        let get = |key| values.get(&key).copied();
        let build = get(BUILD).unwrap_or(1.0);
        let height = get(HEIGHT).unwrap_or(1.0);
        let mut aspects: std::collections::BTreeMap<u32, std::collections::BTreeMap<u32, f32>> =
            Default::default();
        for &(physique, aspect) in self.aspects {
            if let (Some(value), Some(id)) = (get(physique), trait_id(physique)) {
                let traits = aspects.entry(aspect).or_default();
                if value != 0.0 {
                    traits.insert(id, value);
                }
            }
        }
        for &(physique, aspect) in self.clothing {
            let traits = aspects.entry(aspect).or_default();
            if let Some(colour) = get(physique).filter(|value| *value != 0.0) {
                traits.insert(CLOTHING_COLOR, colour);
            }
            traits.insert(CLOTHING_WORN, 1.0);
        }
        aspects.retain(|_, traits| !traits.is_empty());

        let mut bytes = Vec::new();
        bytes.extend(0x122u32.to_le_bytes());
        bytes.extend(self.visual.to_le_bytes());
        for value in [build, build, height] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(0x21u32.to_le_bytes());
        bytes.extend(0x4000_0002u32.to_le_bytes());
        bytes.extend(table_header(aspects.len())?.to_le_bytes());
        for (aspect, traits) in bucket_order(&aspects) {
            bytes.extend(aspect.to_le_bytes());
            bytes.extend(table_header(traits.len())?.to_le_bytes());
            for (id, &value) in bucket_order(traits) {
                bytes.extend(id.to_le_bytes());
                bytes.extend(value.to_le_bytes());
            }
        }
        Ok(bytes)
    }
}

/// Byte length of a VisualDesc this server writes: parent-only (flags 2) or
/// the creation appearance layout (flags 0x122). None for anything else.
pub fn visual_len(bytes: &[u8]) -> Option<usize> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    match word(0)? {
        2 => Some(8),
        0x122 => {
            if word(20)? != 0x21 || word(24)? != 0x4000_0002 {
                return None;
            }
            let aspects = word(28)? & 0xff_ffff;
            let mut at = 32;
            for _ in 0..aspects.min(256) {
                let traits = word(at + 4)? & 0xff_ffff;
                if traits > 256 {
                    return None;
                }
                at += 8 + traits as usize * 8;
            }
            (at <= bytes.len() && aspects <= 256).then_some(at)
        }
        _ => None,
    }
}

/// Retail appearance hashes use bucket-size index 2 (47 buckets, client table
/// 00a7b140); native unpacking needs each bucket's entries to be consecutive.
const BUCKETS: u32 = 47;
fn table_header(count: usize) -> Result<u32, WorldError> {
    if count > 0xff_ffff {
        return Err(WorldError::LimitExceeded);
    }
    Ok(0x0200_0000 | count as u32)
}
fn bucket_order<T>(map: &std::collections::BTreeMap<u32, T>) -> Vec<(u32, &T)> {
    let mut entries: Vec<_> = map.iter().map(|(&key, value)| (key, value)).collect();
    entries.sort_unstable_by_key(|(key, _)| (key % BUCKETS, *key));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(value: f32) -> u32 {
        value.to_bits()
    }

    #[test]
    fn human_female_matches_the_retail_creation_capture() {
        // pkt_2017-1-22_1485082723: CreateCharacter physique values.
        let physical = [
            (1, f(0.85)), (2, f(1.0)), (4, f(0.05)), (0x1000, f(0.0)), (8, f(0.0)),
            (0x10, f(0.25)), (0x40, f(0.3)), (0x100, f(0.96)), (0x800, f(0.11)),
            (0x20, f(0.0)), (0x80, f(0.0)), (0x400, f(0.2)), (0x200, f(0.75)),
        ];
        let desc = body(HUMAN, FEMALE).unwrap().visual_desc(&physical).unwrap();
        // Same capture: the CharGenVerification VisualDesc.
        let expected = concat!(
            "220100002400001f9a99593f9a99593f0000803f2100000002000040060000020f000020",
            "0100000201000000cdcc4c3d1000002002000002030000000000803e0b0000009a99993e",
            "11000020010000021c000041ae47e13d1300002002000002020000000000403f10000000",
            "0000803f1700002002000002020000008fc2753f100000000000803f1900002002000002",
            "02000000cdcc4c3e100000000000803f",
        );
        let mut hex = String::new();
        for byte in &desc {
            hex.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(hex, expected);
    }

    #[test]
    fn unknown_keys_and_out_of_range_values_are_rejected() {
        let human = body(HUMAN, MALE).unwrap();
        assert!(human.visual_desc(&[(0x2000, f(0.5))]).is_err());
        assert!(human.visual_desc(&[(SKIN_TONE, f(1.5))]).is_err());
        assert!(human.visual_desc(&[(BUILD, f(3.0))]).is_err());
        assert!(human.visual_desc(&[(SKIN_TONE, f32::NAN.to_bits())]).is_err());
        assert!(human.visual_desc(&[]).is_ok());
        assert!(body(0x10, MALE).is_none());
    }

    #[test]
    fn every_body_can_receive_its_starter_attacks() {
        let state = crate::progression::Progression::default();
        for body in bodies() {
            let book = crate::skills::Book::default()
                .with_starter_skills(&state, body.race, body.sex)
                .unwrap();
            assert!(book.has_starter_skills(body.race, body.sex));
            assert_eq!(book.with_starter_skills(&state, body.race, body.sex).unwrap(), book);
        }
        let lugian = crate::skills::Book::default();
        assert!(lugian.with_training_attack(&state, LUGIAN).is_err());
    }
}
