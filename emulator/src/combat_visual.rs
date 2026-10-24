//! Narrow training melee presentation from native 7148 physics serializers.
//! DoMode: 00495420; stamped IID: 004946d0; BehaviorParams: 00679220.
//! The human 44000001 combo table resolves one-hand mode + Melee_Auto1.
//! Damage, range and timing remain the authored encounter owner's authority.
use crate::world::WorldError;

pub const DO_MODE: u32 = 0x10082;
pub const DO_BEHAVIOR: u32 = 0x1007f;
pub const PEACE: u32 = 0x40000001;
pub const ONE_HAND: u32 = 0x4100000a;
pub const MELEE_AUTO: u32 = 0x41000089;

fn prefix(opcode: u32, owner: u64, instance: u16, stamp: u16) -> Result<Vec<u8>, WorldError> {
    if [0, u64::MAX].contains(&owner) || instance == 0 || stamp == 0 {
        return Err(WorldError::InvalidValue);
    }
    let mut wire = Vec::with_capacity(24);
    wire.extend(opcode.to_le_bytes());
    wire.extend(owner.to_le_bytes());
    wire.extend(instance.to_le_bytes());
    wire.extend(stamp.to_le_bytes());
    Ok(wire)
}

pub fn mode(owner: u64, instance: u16, stamp: u16, fighting: bool) -> Result<Vec<u8>, WorldError> {
    let mut wire = prefix(DO_MODE, owner, instance, stamp)?;
    wire.extend((if fighting { ONE_HAND } else { PEACE }).to_le_bytes());
    Ok(wire)
}

pub fn swing(owner: u64, instance: u16, stamp: u16) -> Result<Vec<u8>, WorldError> {
    let mut wire = prefix(DO_BEHAVIOR, owner, instance, stamp)?;
    wire.extend(1u32.to_le_bytes()); // Only mBehaviorID; native defaults for all other fields.
    wire.extend(MELEE_AUTO.to_le_bytes());
    Ok(wire)
}
