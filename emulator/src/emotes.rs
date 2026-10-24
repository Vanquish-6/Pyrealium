//! Bounded native emote requests and instance-stamped behavior messages.
//!
//! Wire authority: build-7148 InstanceIDWithStamp::StreamPack (004946d0),
//! BehaviorParams::StreamPack (00679220), defaults (006795b0), and native
//! DoBehavior receive path (00497a00 / 004ec680). The eight token/behavior
//! pairs below were also observed in retained afk.elog, dancing.elog and
//! creating.group.and.inviting.another.member.1.elog captures. Capture client
//! identity is unverified; this is not a claim of matching-client acceptance.
//! Behavior names/IDs are corroborated by the installed 23000027 enum mapper.
//! Emote scope excludes gameplay effects, persistent AFK state and arbitrary
//! client-selected behaviors. No captured context or instance IDs are replayed.

use crate::{dat::Dat, world::WorldError};

pub const REQUEST: u32 = 0x8205000b;
pub const EVENT: u32 = 0x1007f;
/// Explicit server policy; independent of animation duration or native balance.
pub const COOLDOWN_MS: u64 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Definition {
    pub token: u32,
    pub behavior: u32,
    pub name: &'static str,
    pub move_cancels: bool,
    pub effect: Option<u32>,
}

/// Only capture-observed pairings, not every value in the behavior enum.
pub const DEFINITIONS: [Definition; 8] = [
    Definition {
        token: 0xdd5ec189,
        behavior: 0x41000117,
        name: "Wave",
        move_cancels: false,
        effect: None,
    },
    Definition {
        token: 0xdd567195,
        behavior: 0x4100011e,
        name: "Yawn",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0x6173a576,
        behavior: 0x4100001c,
        name: "Moose",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0xce736d78,
        behavior: 0x4100012b,
        name: "Dance 1",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0x0e736d78,
        behavior: 0x4100012c,
        name: "Dance 2",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0x4e736d78,
        behavior: 0x4100012d,
        name: "Dance 3",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0x8e736d79,
        behavior: 0x4100012e,
        name: "Dance 4",
        move_cancels: true,
        effect: None,
    },
    Definition {
        token: 0x32b4a467,
        behavior: 0x41000125,
        name: "AFK",
        move_cancels: true,
        // Native mFxID, not a client-supplied executable/asset reference.
        effect: Some(0x410004fc),
    },
];

pub fn lookup(token: u32) -> Option<&'static Definition> {
    DEFINITIONS
        .iter()
        .find(|definition| definition.token == token)
}

/// The supplied ClientEvent packet profile is exactly one tagged uint32.
pub fn decode(arguments: &[u8]) -> Result<u32, WorldError> {
    if arguments.len() < 8 {
        return Err(WorldError::Truncated);
    }
    if arguments.len() > 8 {
        return Err(WorldError::TrailingBytes);
    }
    let tag = u32::from_le_bytes(arguments[..4].try_into().unwrap());
    let token = u32::from_le_bytes(arguments[4..].try_into().unwrap());
    if tag != 1 || lookup(token).is_none() {
        return Err(WorldError::InvalidValue);
    }
    Ok(token)
}

/// Build a fresh native behavior event for the active player's current lifetime.
pub fn encode(
    owner: u64,
    instance: u16,
    behavior_stamp: u16,
    token: u32,
) -> Result<Vec<u8>, WorldError> {
    if [0, u64::MAX].contains(&owner) || instance == 0 || behavior_stamp == 0 {
        return Err(WorldError::InvalidValue);
    }
    let definition = lookup(token).ok_or(WorldError::InvalidValue)?;
    // Native default hold count is one. Avoid captured context counters: their
    // lifecycle is not implemented, and no callback acknowledgement is needed.
    // FadeChildren is the observed weapon-visibility behavior while emoting.
    let flags: u32 = 1
        | 0x100
        | 0x800000
        | if definition.move_cancels { 0x20 } else { 0 }
        | if definition.effect.is_some() {
            0x100000
        } else {
            0
        };
    let mut wire = Vec::with_capacity(36);
    wire.extend(EVENT.to_le_bytes());
    wire.extend(owner.to_le_bytes());
    wire.extend(instance.to_le_bytes());
    wire.extend(behavior_stamp.to_le_bytes());
    wire.extend(flags.to_le_bytes());
    wire.extend(definition.behavior.to_le_bytes());
    if let Some(effect) = definition.effect {
        wire.extend(effect.to_le_bytes());
    }
    if definition.move_cancels {
        wire.extend(1u32.to_le_bytes());
    }
    wire.extend(1u32.to_le_bytes()); // mFadeChildren
    wire.extend(definition.token.to_le_bytes()); // mWeenieEmoteID
    Ok(wire)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    instance: u16,
    stamp: u16,
    last_ms: Option<u64>,
}

impl State {
    /// Shared checked visual order for emotes and combat physics messages.
    /// Return a candidate; callers publish it only after outgoing admission.
    pub fn next_visual(self, instance: u16) -> Result<(Self, u16), WorldError> {
        if instance == 0 {
            return Err(WorldError::InvalidValue);
        }
        let previous = if instance == self.instance {
            self.stamp
        } else {
            0
        };
        let stamp = previous.checked_add(1).ok_or(WorldError::LimitExceeded)?;
        Ok((
            Self {
                instance,
                stamp,
                ..self
            },
            stamp,
        ))
    }
    /// Candidate only: publish this returned state after outgoing admission.
    /// The server must independently require ownership, ready visibility, a
    /// living character and its chosen out-of-combat policy before calling.
    pub fn prepare(
        self,
        owner: u64,
        instance: u16,
        now_ms: u64,
        token: u32,
    ) -> Result<(Self, Vec<u8>), WorldError> {
        if self
            .last_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < COOLDOWN_MS)
        {
            return Err(WorldError::LimitExceeded);
        }
        let (next, stamp) = self.next_visual(instance)?;
        let wire = encode(owner, instance, stamp, token)?;
        Ok((
            Self {
                last_ms: Some(now_ms),
                ..next
            },
            wire,
        ))
    }
}

/// Pin the enum mapper and supported default-human behavior table. This is a
/// compatibility check, not authentication or proof of native appearance.
pub fn validate_assets(portal: &mut Dat) -> std::io::Result<()> {
    for (did, expected) in [
        (0x23000027, 0x9afe79457af1c070u64),
        (0x44000001, 0xba2919482be463eau64),
    ] {
        let record = portal.read(did)?;
        let actual = record.iter().fold(0xcbf29ce484222325u64, |hash, value| {
            (hash ^ u64::from(*value)).wrapping_mul(0x100000001b3)
        });
        if actual != expected {
            return Err(std::io::Error::other(format!(
                "emote definition asset differs from supported metadata: {did:#010x}"
            )));
        }
    }
    Ok(())
}
