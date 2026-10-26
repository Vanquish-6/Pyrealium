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
    /// Complete quantity requested by the native drag; never a split amount.
    pub quantity: u16,
}
impl MoveRequest {
    pub fn arguments(bytes: &[u8]) -> Result<Self, WorldError> {
        if bytes.len() != MESSAGE_BYTES - 12 {
            return Err(WorldError::InvalidValue);
        }
        let mut message = Vec::with_capacity(MESSAGE_BYTES);
        for word in [0x10053u32, MOVE, bytes.len() as u32] {
            message.extend(word.to_le_bytes());
        }
        message.extend(bytes);
        Self::decode(&message)
    }
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
            || message[295] != 0
        {
            return Err(WorldError::InvalidValue);
        }
        let fields: [u32; 51] = std::array::from_fn(|i| word(40 + i * 4));
        let mut expected_tags = TAGS;
        // Captured drag requests use a uint source ordinal when nonzero and
        // a uint whole-stack count instead of the single-item constant.
        if !matches!(message[245], 2 | 4) {
            return Err(WorldError::InvalidValue);
        }
        expected_tags[1] = message[245];
        if fields[25] > 1 { expected_tags[25] = 4; }
        if message[244..295] != expected_tags {
            return Err(WorldError::InvalidValue);
        }
        let owner = u64::from(fields[2]) | (u64::from(fields[3]) << 32);
        let item = u64::from(fields[28]) | (u64::from(fields[29]) << 32);
        if [0, u64::MAX].contains(&owner)
            || [0, u64::MAX].contains(&item)
            || owner == item
            || fields[1] >= u32::from(BAG_SLOTS)
            || fields[11] > u32::from(BAG_SLOTS)
            || fields[25] == 0
            || fields[25] > u32::from(u16::MAX)
            || fields[25] != fields[26]
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
        expected[25] = fields[25];
        expected[26] = fields[26];
        if fields != expected {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            owner,
            item,
            source: fields[1] as u16,
            destination: fields[11] as u16,
            quantity: fields[25] as u16,
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
    pub fn checked_command(
        &self,
        owner: u64,
        inventory: &crate::inventory::Inventory,
    ) -> Result<Command, WorldError> {
        if !inventory.items().any(|item| {
            item.id == self.item
                && item.quantity == self.quantity
                && item.quantity > 0
                && item.location == crate::inventory::Location::Bag(self.source)
        }) {
            return Err(WorldError::InvalidValue);
        }
        self.command(owner)
    }

    /// Same-IID whole-item move acknowledgment, including a complete stack.
    /// Optional per-item graphs and partial-stack movement remain unsupported.
    pub fn acknowledgment(
        &self,
        item: &crate::inventory::Item,
        metadata: Metadata,
    ) -> Result<Vec<u8>, WorldError> {
        if item.id != self.item || item.quantity != self.quantity || item.quantity == 0 {
            return Err(WorldError::InvalidValue);
        }
        let crate::inventory::Location::Bag(final_slot) = item.location else {
            return Err(WorldError::InvalidValue);
        };
        let mut fields = [0u32; 51];
        for index in [7, 10, 13, 23, 26, 35, 47] {
            fields[index] = 1;
        }
        for index in [2, 8, 16, 32, 36] {
            fields[index] = self.owner as u32;
            fields[index + 1] = (self.owner >> 32) as u32;
        }
        for index in [28, 44] {
            fields[index] = item.id as u32;
            fields[index + 1] = (item.id >> 32) as u32;
        }
        fields[1] = u32::from(self.source);
        fields[39] = u32::from(self.source);
        fields[11] = u32::from(self.destination);
        fields[24] = u32::from(self.destination);
        fields[50] = u32::from(final_slot);
        fields[6] = metadata.icon;
        fields[22] = item.definition;
        fields[30] = u32::MAX;
        fields[38] = metadata.physical;
        fields[26] = u32::from(item.quantity);
        if item.quantity > 1 {
            fields[48] = u32::from(item.quantity);
        }
        let mut arguments = Vec::with_capacity(MESSAGE_BYTES - 12);
        for word in [5, 1, 1, 0x44000100, 0, 0x01f80000, 51] {
            arguments.extend(u32::to_le_bytes(word));
        }
        for word in fields {
            arguments.extend(word.to_le_bytes());
        }
        let mut tags = TAGS;
        if item.quantity > 1 { tags[48] = 4; }
        tags[50] = if self.source < self.destination { 2 } else { 4 };
        arguments.extend(tags);
        arguments.push(0);
        crate::script::private_call(0x820e0055, &arguments)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Metadata {
    /// Legacy field name: DID quality 103 / inventory AppearanceFile, not the
    /// UI descriptor's texture icon. Zero denotes an absent appearance profile.
    pub icon: u32,
    pub physical: u32,
    pub equipment: Option<Equipment>,
}
#[derive(Clone, Copy, Debug)]
pub struct Equipment {
    pub valid: u32,
    pub preferred: u32,
    pub primary: u32,
    pub secondary: u32,
}
impl Equipment {
    pub fn hand_slot(self, item: &crate::inventory::Item) -> Option<u16> {
        let slot = match item.location {
            crate::inventory::Location::Equipped(slot) => u32::from(slot),
            _ => self.preferred,
        };
        // Explicit initial hand-equipment policy. Body armor needs VisualDesc
        // composition. Arbitrary parenting selectors are never accepted.
        if !matches!(slot, 0x40 | 0x80)
            || self.valid & slot == 0
            || u32::from(item.equipment_mask) & slot == 0
            || !matches!(self.primary, 1 | 2)
            || (slot == 0x40 && self.primary != 1)
            || (slot == 0x80 && self.primary != 2)
            || item.quantity != 1
        {
            return None;
        }
        Some(slot as u16)
    }
}
impl Metadata {
    /// Read only the authoritative qualities prefix needed by bag acknowledgments.
    /// gmWeenieDesc::StreamPack 006037c0, CBaseQualities::UnPack 005192f0.
    /// Localized names with substitutions and string quality tables are unsupported.
    pub fn load(portal: &mut crate::dat::Dat, definition: u32) -> std::io::Result<Self> {
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported item metadata layout",
            )
        };
        if definition >> 24 != 0x47 {
            return Err(invalid());
        }
        let did = 0x81000000 | (definition & 0xffffff);
        let bytes = portal.read(did)?;
        if bytes.len() > 65536 {
            return Err(invalid());
        }
        let mut pos = 0usize;
        let mut word = || -> std::io::Result<u32> {
            let value = bytes.get(pos..pos + 4).ok_or_else(invalid)?;
            pos += 4;
            Ok(u32::from_le_bytes(value.try_into().unwrap()))
        };
        let mask = word()?;
        if mask & 0x21 != 0x21 || mask & !0xfff != 0 || mask & 0x80 != 0 || word()? != did {
            return Err(invalid());
        }
        let desc = word()?;
        if desc & !0x7fffffff != 0 || desc & 0x10 == 0 {
            return Err(invalid());
        }
        for bit in [1, 0x2000000, 2] {
            if desc & bit != 0 {
                word()?;
            }
        }
        for bit in [4, 8] {
            if desc & bit != 0 {
                let table = word()?;
                word()?;
                if table == 0 || word()? != 0 {
                    return Err(invalid());
                }
            }
        }
        word()?; // Description icon selector; the move reply uses DID quality 103.
        for (bit, words) in [
            (0x20, 2),
            (0x40, 2),
            (0x100, 2),
            (0x200, 2),
            (0x20000000, 2),
            (0x40000000, 2),
            (0x4000000, 2),
            (0x80, 1),
            (0x800, 1),
            (0x1000, 1),
            (0x2000, 1),
            (0x200000, 1),
            (0x400000, 1),
            (0x8000, 1),
            (0x10000, 1),
            (0x20000, 1),
            (0x40000, 1),
            (0x80000, 1),
            (0x100000, 1),
            (0x8000000, 1),
            (0x10000000, 1),
        ] {
            if desc & bit != 0 {
                for _ in 0..words {
                    word()?;
                }
            }
        }
        // Reject currently unassigned description bits rather than offset guessing.
        const DESC_BITS: u32 = 1
            | 0x2000000
            | 2
            | 4
            | 8
            | 0x10
            | 0x20
            | 0x40
            | 0x100
            | 0x200
            | 0x20000000
            | 0x40000000
            | 0x4000000
            | 0x80
            | 0x800
            | 0x1000
            | 0x2000
            | 0x200000
            | 0x400000
            | 0x8000
            | 0x10000
            | 0x20000
            | 0x40000
            | 0x80000
            | 0x100000
            | 0x8000000
            | 0x10000000;
        if desc & !DESC_BITS != 0 {
            return Err(invalid());
        }
        let mut integers = std::collections::BTreeMap::new();
        for (bit, width) in [(2, 2), (4, 2), (8, 2), (0x10, 3)] {
            if mask & bit != 0 {
                let header = word()?;
                let count = header & 0xffffff;
                if header >> 24 > 22 || count > 512 {
                    return Err(invalid());
                }
                let mut keys = std::collections::BTreeSet::new();
                for _ in 0..count {
                    let key = word()?;
                    let value = word()?;
                    if !keys.insert(key) {
                        return Err(invalid());
                    }
                    if bit == 2 {
                        integers.insert(key, value);
                    }
                    for _ in 2..width {
                        word()?;
                    }
                }
            }
        }
        let header = word()?;
        let count = header & 0xffffff;
        if header >> 24 > 22 || count > 512 {
            return Err(invalid());
        }
        let mut physical = None;
        let mut icon = None;
        let mut keys = std::collections::BTreeSet::new();
        for _ in 0..count {
            let key = word()?;
            let value = word()?;
            if !keys.insert(key) {
                return Err(invalid());
            }
            if key == 1 {
                physical = Some(value);
            }
            if key == 0x103 {
                icon = Some(value);
            }
        }
        let physical = physical.ok_or_else(invalid)?;
        let icon = icon.ok_or_else(invalid)?;
        if icon >> 24 != 0x20 || physical >> 24 != 0x47 {
            return Err(invalid());
        }
        portal.read(icon)?;
        portal.read(physical)?;
        let equipment = (|| {
            Some(Equipment {
                valid: *integers.get(&0x113)?,
                preferred: *integers.get(&0x11a)?,
                primary: *integers.get(&0x114)?,
                secondary: *integers.get(&0x115)?,
            })
        })();
        Ok(Self {
            icon,
            physical,
            equipment,
        })
    }
}
