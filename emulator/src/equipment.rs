//! Narrow captured object-use profile for equipment and lifestones. No VM execution.
use crate::world::WorldError;

pub const USE: u32 = 0x82160016;

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
