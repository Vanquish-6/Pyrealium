//! Narrow captured object-use profile for equipment and lifestones. No VM execution.
use crate::world::WorldError;

pub const USE: u32 = 0x82160016;
pub const UNEQUIP: u32 = 0x820e0015;
pub const EQUIP: u32 = 0x820e004f;

/// Native drag from the player's backpack onto an equipment location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipRequest {
    pub owner: u64,
    pub item: u64,
    pub slot: u32,
    pub source: u32,
}
impl EquipRequest {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        // gmCWInterface::EquipItem 004504f0 / glue 00459140 -> b4000d
        // (object 1aa4, code 98f50) constructs 1f6 using 1f6002b.
        // 9ab40016 records source index using 1f6001f, then 820e0059
        // (1005c0) packs the graph for client event 820e004f.
        if arguments.len() != 180 {
            return Err(WorldError::InvalidValue);
        }
        let word =
            |offset: usize| u32::from_le_bytes(arguments[offset..offset + 4].try_into().unwrap());
        let first = word(8);
        let second = word(12);
        if first == second
            || [0, u32::MAX].contains(&first)
            || [0, u32::MAX].contains(&second)
            || [word(0), word(4), word(16), word(20), word(24), word(28)]
                != [5, 2, 0x44000100, 0, 0x01f60000, 19]
            || [word(128), word(132), word(136), word(140)] != [0x44000100, 0, 0x01f50000, 7]
        {
            return Err(WorldError::InvalidValue);
        }
        let fields: [u32; 19] = std::array::from_fn(|index| word(32 + index * 4));
        let item = u64::from(fields[3]) | (u64::from(fields[4]) << 32);
        let owner = u64::from(fields[8]) | (u64::from(fields[9]) << 32);
        let slot = fields[7];
        let source = fields[18];
        if [0, u64::MAX].contains(&item)
            || [0, u64::MAX].contains(&owner)
            || item == owner
            || [0, u32::MAX].contains(&slot)
            || source >= u32::from(crate::inventory::BAG_SLOTS)
        {
            return Err(WorldError::InvalidValue);
        }
        let mut expected = [0; 19];
        expected[1] = second;
        expected[2] = 1;
        for index in [3, 4, 8, 9] {
            expected[index] = fields[index];
        }
        expected[7] = slot;
        // The client finds the source container/index before sending. Support
        // only direct ownership in our player's backpack; no nested containers.
        expected[10] = fields[8];
        expected[11] = fields[9];
        expected[12] = 0x9e;
        expected[15] = slot;
        expected[18] = source;
        const TAGS: [u8; 19] = [
            2, 1, 2, 0x84, 0xcd, 2, 2, 4, 0x84, 0xcd, 0x84, 0xcd, 2, 2, 2, 4, 0x82, 0xcd, 4,
        ];
        if fields != expected
            || arguments[108..127] != TAGS
            || arguments[127] != 0
            || arguments[144..172].iter().any(|&byte| byte != 0)
            || arguments[172..179] != [2; 7]
            || arguments[179] != 0
        {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            owner,
            item,
            slot,
            source,
        })
    }
}

/// Native equipment-panel drag to a backpack position, distinct from Use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnEquipRequest {
    pub owner: u64,
    pub item: u64,
    pub slot: u32,
    pub destination: u32,
}
impl UnEquipRequest {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        // Pinned 7148 b4000e (object 1aa4, code 98fc4) constructs action
        // 1f6 via 1f6002b, sets destination through 1f60030/1f6001f,
        // then 9ab4000a -> 820e005b packs it for event 820e0015.
        // Both constructors (81f6002c / 81f50012) establish the zero fields.
        if arguments.len() != 180 {
            return Err(WorldError::InvalidValue);
        }
        let word =
            |offset: usize| u32::from_le_bytes(arguments[offset..offset + 4].try_into().unwrap());
        let first = word(8);
        let second = word(12);
        if first == second
            || [0, u32::MAX].contains(&first)
            || [0, u32::MAX].contains(&second)
            || [word(0), word(4), word(16), word(20), word(24), word(28)]
                != [5, 2, 0x44000100, 0, 0x01f60000, 19]
            || [word(128), word(132), word(136), word(140)] != [0x44000100, 0, 0x01f50000, 7]
        {
            return Err(WorldError::InvalidValue);
        }
        let fields: [u32; 19] = std::array::from_fn(|index| word(32 + index * 4));
        let item = u64::from(fields[3]) | (u64::from(fields[4]) << 32);
        let owner = u64::from(fields[8]) | (u64::from(fields[9]) << 32);
        let slot = fields[7];
        // This is a destination index, not an equipment slot or quantity:
        // gmBackpackUI::ListenToMessage 004414e0 passes the selected bag index.
        let destination = fields[6];
        if [0, u64::MAX].contains(&item)
            || [0, u64::MAX].contains(&owner)
            || item == owner
            || [0, u32::MAX].contains(&slot)
            || destination > i32::MAX as u32
        {
            return Err(WorldError::InvalidValue);
        }
        let mut expected = [0; 19];
        expected[1] = second;
        expected[2] = 1;
        for index in [3, 4, 8, 9] {
            expected[index] = fields[index];
        }
        expected[6] = destination;
        expected[7] = slot;
        expected[10] = fields[8];
        expected[11] = fields[9];
        expected[12] = 0x9e;
        expected[15] = slot;
        expected[18] = destination;
        const TAGS: [u8; 19] = [
            2, 1, 2, 0x84, 0xcd, 2, 4, 4, 0x84, 0xcd, 0x84, 0xcd, 2, 2, 2, 4, 0x82, 0xcd, 4,
        ];
        if fields != expected
            || arguments[108..127] != TAGS
            || arguments[127] != 0
            || arguments[144..172].iter().any(|&byte| byte != 0)
            || arguments[172..179] != [2; 7]
            || arguments[179] != 0
        {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            owner,
            item,
            slot,
            destination,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseRequest {
    pub owner: u64,
    pub item: u64,
}
impl UseRequest {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        if arguments.len() != 160 {
            return Err(WorldError::InvalidValue);
        }
        let word =
            |offset: usize| u32::from_le_bytes(arguments[offset..offset + 4].try_into().unwrap());
        let first = word(8);
        let second = word(12);
        if word(0) != 5
            || word(4) != 2
            || first == second
            || [0, u32::MAX].contains(&first)
            || [0, u32::MAX].contains(&second)
            || word(16) != 0x44000100
            || word(20) != 0
            || word(24) != 0x02450000
            || word(28) != 20
        {
            return Err(WorldError::InvalidValue);
        }
        let fields: [u32; 20] = std::array::from_fn(|index| word(32 + index * 4));
        let item = u64::from(fields[2]) | (u64::from(fields[3]) << 32);
        let owner = u64::from(fields[6]) | (u64::from(fields[7]) << 32);
        if [0, u64::MAX].contains(&item) || [0, u64::MAX].contains(&owner) || item == owner {
            return Err(WorldError::InvalidValue);
        }
        let mut expected = [0; 20];
        expected[0] = second;
        for index in [1, 4, 15] {
            expected[index] = u32::MAX;
        }
        for index in [2, 3, 6, 7] {
            expected[index] = fields[index];
        }
        expected[8] = (-1f32).to_bits();
        expected[17] = 0x13c;
        const TAGS: [u8; 20] = [
            1, 1, 0x84, 0xcd, 1, 2, 0x84, 0xcd, 3, 0x82, 0xcd, 2, 0x82, 0xcd, 2, 1, 2, 2, 2, 2,
        ];
        if fields != expected || arguments[112..132] != TAGS
            || word(132) != 0x44000100 || word(136) != 0 || word(140) != 0xffff006a
            // A localized display hint, never an authorization or execution input.
            || word(148) >> 24 != 0x25 || word(152) != 0 || word(156) != 0
        {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self { owner, item })
    }
}
