//! Bounded inventory sorting request profile from retained packet captures.
//! Parses data only; does not instantiate interpreted objects or execute scripts.
use crate::{
    inventory::{BAG_SLOTS, Command},
    world::WorldError,
};

pub const MOVE: u32 = 0x820e0049;
pub const MESSAGE_BYTES: usize = 296;
const TAGS: [u8; 51] = [
    2, 2, 0x84, 0xcd, 2, 2, 4, 2, 0x84, 0xcd, 2, 4, 2, 2, 2, 2, 0x84, 0xcd, 0x82, 0xcd, 2, 2, 4, 2,
    4, 2, 4, 2, 0x84, 0xcd, 1, 2, 0x84, 0xcd, 2, 2, 0x84, 0xcd, 4, 4, 2, 2, 2, 2, 0x84, 0xcd, 2, 2,
    2, 2, 4,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveRequest {
    pub owner: u64,
    pub item: u64,
    pub source: u16,
    pub destination: u16,
}
impl MoveRequest {
    /// Input is one reassembled application message, without transport headers.
    pub fn decode(message: &[u8]) -> Result<Self, WorldError> {
        if message.len() != MESSAGE_BYTES {
            return Err(WorldError::InvalidValue);
        }
        let word =
            |offset: usize| u32::from_le_bytes(message[offset..offset + 4].try_into().unwrap());
        if word(0) != 0x10053
            || word(4) != MOVE
            || word(8) != 284
            || word(12) != 5
            || word(16) != 1
            || word(20) == u32::MAX
            || word(24) != 0x44000100
            || word(28) != 0
            || word(32) != 0x01f80000
            || word(36) != 51
            || message[244..295] != TAGS
            || message[295] != 0
        {
            return Err(WorldError::InvalidValue);
        }
        let fields: [u32; 51] = std::array::from_fn(|i| word(40 + i * 4));
        let owner = u64::from(fields[2]) | (u64::from(fields[3]) << 32);
        let item = u64::from(fields[28]) | (u64::from(fields[29]) << 32);
        if [0, u64::MAX].contains(&owner)
            || [0, u64::MAX].contains(&item)
            || owner == item
            || fields[1] >= u32::from(BAG_SLOTS)
            || fields[11] > u32::from(BAG_SLOTS)
        {
            return Err(WorldError::InvalidValue);
        }
        // Verify repeated identities, positions, discriminants and absent optional
        // metadata. No client-provided definition/icon/appearance becomes authority.
        let mut expected = [0u32; 51];
        for index in [0, 10, 13, 23, 25, 26, 35, 47, 49] {
            expected[index] = 1;
        }
        for index in [2, 8, 16, 32, 36] {
            expected[index] = owner as u32;
            expected[index + 1] = (owner >> 32) as u32;
        }
        for index in [28, 44] {
            expected[index] = item as u32;
            expected[index + 1] = (item >> 32) as u32;
        }
        expected[1] = fields[1];
        expected[39] = fields[1];
        for index in [11, 24, 50] {
            expected[index] = fields[11];
        }
        expected[30] = u32::MAX;
        if fields != expected {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            owner,
            item,
            source: fields[1] as u16,
            destination: fields[11] as u16,
        })
    }
    pub fn command(&self, owner: u64) -> Result<Command, WorldError> {
        if self.owner != owner {
            return Err(WorldError::InvalidValue);
        }
        Ok(Command::Reorder {
            item: self.item,
            source: self.source,
            destination: self.destination,
        })
    }
}
