//! Narrow native NPC-use completion and speech profiles. No VM execution.
//!
//! Evidence: cleaned Npc interaction merchant-hello capture, messages 4/5;
//! SHA256 101b39cdaee4a2fd618fb97b85914ba4419ccb5fdeb2b992ec6708e18b9dc915.
//! 26 NPC completions share the successful 245 profile below. The observed
//! alternate flags 3a0 are not assigned a guessed failure meaning here.
use crate::{movement::Placement, script, world::WorldError};

pub const USE_RESULT: u32 = 0x82160021;
pub const NPC_SPEECH: u32 = 0x82050017;
pub const LAYOUTS: [(usize, u32); 1] = [(0x245, 20)];

fn valid_id(id: u64) -> bool {
    id != 0 && id != u64::MAX
}
fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}
fn object_header(bytes: &mut Vec<u8>, kind: u32) {
    for value in [0x44000100, 0, kind] {
        word(bytes, value);
    }
}

/// Complete a successful, server-authorized NPC use. Position and distance
/// must come from authoritative world state, never from the request graph.
/// `point` is the owner's current placement, not the NPC's position.
/// This sends the captured success profile, not a failure or a teleport.
pub fn success(
    owner: u64,
    target: u64,
    point: Placement,
    distance: f32,
) -> Result<Vec<u8>, WorldError> {
    if !valid_id(owner)
        || !valid_id(target)
        || owner == target
        || !point.valid()
        || !distance.is_finite()
        || !(0.0..=64.0).contains(&distance)
    {
        return Err(WorldError::InvalidValue);
    }
    // Fresh local graph references; no retained packet callback/context IDs.
    let mut args = Vec::with_capacity(212);
    for value in [5, 3, 1, 2, 3] {
        word(&mut args, value);
    }
    object_header(&mut args, 0x02450000);
    word(&mut args, 20);
    let mut fields = [0u32; 20];
    fields[0] = 2;
    fields[1] = u32::MAX;
    fields[2] = target as u32;
    fields[3] = (target >> 32) as u32;
    fields[4] = 3;
    fields[5] = 0x1f;
    fields[6] = owner as u32;
    fields[7] = (owner >> 32) as u32;
    fields[8] = distance.to_bits();
    fields[12] = owner as u32;
    fields[13] = (owner >> 32) as u32;
    fields[15] = u32::MAX;
    fields[17] = 0x3bc;
    for value in fields {
        word(&mut args, value);
    }
    args.extend([
        1, 1, 0x84, 0xcd, 1, 4, 0x84, 0xcd, 4, 0x82, 0xcd, 2, 0x84, 0xcd, 4, 1, 4, 2, 4, 4,
    ]);
    object_header(&mut args, 0xffff006a);
    // Captured localized action label, not executable client-supplied text.
    for value in [0x4e1aec40, 0x25000014, 0, 0] {
        word(&mut args, value);
    }
    object_header(&mut args, 0xffff005a);
    word(&mut args, point.cell);
    for value in point.coordinates.into_iter().chain(point.orientation()) {
        word(&mut args, value.to_bits());
    }
    word(&mut args, 0);
    script::private_call(USE_RESULT, &args)
}

/// Native speech presentation with an explicit speaker identity and name.
/// This is not a branching conversation or a dialog-choice protocol.
pub fn speech(speaker: u64, name: &str, text: &str) -> Result<Vec<u8>, WorldError> {
    if !valid_id(speaker)
        || name.is_empty()
        || name.trim() != name
        || name.encode_utf16().count() > 32
        || text.is_empty()
        || text.encode_utf16().count() > 512
        || name.chars().chain(text.chars()).any(char::is_control)
    {
        return Err(WorldError::InvalidValue);
    }
    // 82050017 -> 1aa70013 -> 1aa70002 in the pinned 7148 WLib:
    // integer mode, speech StringInfo, speaker-name StringInfo, speaker IID.
    let mut args = script::integer(0);
    args.extend(script::literal(text)?);
    args.extend(script::literal(name)?);
    args.extend(script::identity(speaker));
    script::private_call(NPC_SPEECH, &args)
}

/// Native 7148 MessageDialog (OK closes locally, no fabricated response opcode).
/// 82120024 -> WLib 10139c / 1aa400a9 -> EFun 85000c -> 00469c10
/// -> CM_Character::SendNotice_DisplayMessage 00724be0 -> 00408080.
/// Capture: händler_gilden_schatzmeister_quitungs_kauf.elog, message SHA256
/// 747db0458f7f42ced721512246d8aa14344dfbbb382ff11b31916e02b6d6ac3a.
pub fn dialogue(speaker: &str, text: &str) -> Result<Vec<u8>, WorldError> {
    if speaker.is_empty() || speaker.encode_utf16().count() > 32
        || text.is_empty() || text.encode_utf16().count() > 1024
        || speaker.chars().chain(text.chars()).any(char::is_control) {
        return Err(WorldError::InvalidValue);
    }
    // The captured argument order is priority integer first, then StringInfo.
    let mut args = script::integer(0);
    args.extend(script::literal(&format!("{speaker}: {text}"))?);
    script::private_call(0x82120024, &args)
}
