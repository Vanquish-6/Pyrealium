//! Narrow training melee presentation from native 7148 physics serializers.
//! DoMode: 00495420; stamped IID: 004946d0; BehaviorParams: 00679220.
//! The human 44000001 combo table resolves one-hand mode + Melee_Auto1.
//! Damage, range and timing remain the authored encounter owner's authority.
use crate::world::WorldError;

pub const DO_MODE: u32 = 0x10082;
pub const DO_BEHAVIOR: u32 = 0x1007f;
/// PhysicsStory on the victim. StreamPack 006ae550; hook pack 006ad9f0.
pub const DO_STORY: u32 = 0x100a8;
pub const DO_FX: u32 = 0x100a0;
pub const PEACE: u32 = 0x40000001;
pub const ONE_HAND: u32 = 0x4100000a;
pub const MELEE_AUTO: u32 = 0x41000089;
/// Captured result paired with a nonzero health hook. Not a reconstructed crit table.
pub const HEALTH_RESULT: u32 = 0x40000008;

#[derive(Clone, Copy)]
pub enum IdolAction {
    Combat,
    Swing,
    Peace,
    Death,
}

/// Exact Idol Spawn profile: BehaviorTable 44000018 and retained fight capture.
/// Return the final shared visual stamp, snapshot mode and fresh wire messages.
pub fn idol(
    owner: u64,
    instance: u16,
    current_stamp: u16,
    action: IdolAction,
) -> Result<(u16, u32, Vec<Vec<u8>>), WorldError> {
    let final_stamp = current_stamp
        .checked_add(if matches!(action, IdolAction::Swing) {
            2
        } else {
            1
        })
        .ok_or(WorldError::LimitExceeded)?;
    let mode = match action {
        IdolAction::Combat | IdolAction::Swing => 0x41000012,
        IdolAction::Peace => PEACE,
        IdolAction::Death => 0x41000005,
    };
    let mut events = Vec::new();
    if matches!(action, IdolAction::Death) {
        let mut wire = prefix(DO_BEHAVIOR, owner, instance, final_stamp)?;
        wire.extend(3u32.to_le_bytes()); // behavior and explicit mode, no captured context.
        wire.extend(0x4100001fu32.to_le_bytes());
        wire.extend(mode.to_le_bytes());
        events.push(wire);
    } else {
        let mut wire = prefix(DO_MODE, owner, instance, current_stamp + 1)?;
        wire.extend(mode.to_le_bytes());
        events.push(wire);
        if matches!(action, IdolAction::Swing) {
            events.push(swing(owner, instance, final_stamp)?);
        }
    }
    Ok((final_stamp, mode, events))
}

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

/// Player basic-attack presentation. Captured sword attacks use an attacker
/// PhysicsStory, whose native StoryManager creates a fresh behavior context.
/// Damage remains in the existing authoritative encounter/victim update path.
pub fn basic_attack(owner: u64, instance: u16, stamp: u16) -> Result<Vec<u8>, WorldError> {
    let mut wire = prefix(DO_STORY, owner, instance, stamp)?;
    wire.extend(0x26u32.to_le_bytes()); // Attacker, behavior, basic-attack flag.
    wire.extend(owner.to_le_bytes());
    wire.extend(0x601u32.to_le_bytes()); // Behavior, propagate, send-event.
    wire.extend(MELEE_AUTO.to_le_bytes());
    wire.extend(1u32.to_le_bytes()); // Propagate to the held weapon.
    wire.extend(0u32.to_le_bytes()); // No interpreted completion callback.
    wire.extend(0u32.to_le_bytes()); // No duplicated damage hooks.
    Ok(wire)
}

pub fn swing(owner: u64, instance: u16, stamp: u16) -> Result<Vec<u8>, WorldError> {
    behavior(owner, instance, stamp, MELEE_AUTO)
}

/// DoBehavior with only mBehaviorID set; native defaults for all other fields.
pub fn behavior(owner: u64, instance: u16, stamp: u16, id: u32) -> Result<Vec<u8>, WorldError> {
    if id >> 24 != 0x41 {
        return Err(WorldError::InvalidValue);
    }
    let mut wire = prefix(DO_BEHAVIOR, owner, instance, stamp)?;
    wire.extend(1u32.to_le_bytes());
    wire.extend(id.to_le_bytes());
    Ok(wire)
}

/// DoFX on an object: FX id and scale, as in retained captures (0x100a0).
pub fn fx(owner: u64, instance: u16, stamp: u16, id: u32) -> Result<Vec<u8>, WorldError> {
    if id >> 24 != 0x41 {
        return Err(WorldError::InvalidValue);
    }
    let mut wire = prefix(DO_FX, owner, instance, stamp)?;
    wire.extend(id.to_le_bytes());
    wire.extend(1.0f32.to_le_bytes());
    Ok(wire)
}

/// Victim-owned health story. Bitfield 2 carries the attacker and omits skill,
/// behavior and the basic-attack bit. The single hook is type 1 (SetTypeToDamage,
/// 006ad840) with target, result and signed health. StoryManager::DoStoryHook
/// 00514ff0 presents type 1; it does not replace the separate vital-quality update.
/// Negative health is damage. Positive health is recovery. Zero is not packed.
pub fn damage_story(
    victim: u64,
    instance: u16,
    stamp: u16,
    attacker: u64,
    health_change: i32,
) -> Result<Vec<u8>, WorldError> {
    if [0, u64::MAX].contains(&attacker) || health_change == 0 {
        return Err(WorldError::InvalidValue);
    }
    let mut wire = prefix(DO_STORY, victim, instance, stamp)?;
    wire.extend(2u32.to_le_bytes());
    wire.extend(attacker.to_le_bytes());
    wire.extend(1u32.to_le_bytes());
    wire.extend(0x981u32.to_le_bytes());
    wire.extend(victim.to_le_bytes());
    wire.extend(HEALTH_RESULT.to_le_bytes());
    wire.extend(health_change.to_le_bytes());
    Ok(wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_story_matches_the_captured_victim_hook() {
        let wire = damage_story(0x11, 2, 9, 0x22, -20).unwrap();
        assert_eq!(wire.len(), 52);
        assert_eq!(&wire[..4], &DO_STORY.to_le_bytes());
        assert_eq!(&wire[4..12], &0x11u64.to_le_bytes());
        assert_eq!(&wire[12..14], &2u16.to_le_bytes());
        assert_eq!(&wire[14..16], &9u16.to_le_bytes());
        assert_eq!(&wire[16..20], &2u32.to_le_bytes());
        assert_eq!(&wire[20..28], &0x22u64.to_le_bytes());
        assert_eq!(&wire[28..32], &1u32.to_le_bytes());
        assert_eq!(&wire[32..36], &0x981u32.to_le_bytes());
        assert_eq!(&wire[36..44], &0x11u64.to_le_bytes());
        assert_eq!(&wire[44..48], &HEALTH_RESULT.to_le_bytes());
        assert_eq!(&wire[48..52], &(-20i32).to_le_bytes());
        let heal = damage_story(0x11, 2, 10, 0x11, 25).unwrap();
        assert_eq!(&heal[48..52], &25i32.to_le_bytes());
        assert!(damage_story(0x11, 2, 10, 0x22, 0).is_err());
        assert!(damage_story(0, 2, 10, 0x22, -1).is_err());
        assert!(damage_story(0x11, 2, 10, 0, -1).is_err());
    }
}
