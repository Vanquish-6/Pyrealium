//! Durable authored development quest state and native build-7148 journal data.
//!
//! Native layout: gmQuestInfo::StreamPack 0060af00 and PList<gmQuestInfo>::Pack
//! 0060b500. Status semantics: gmQuestInfo::InqExtraText 0060aca0. The training
//! objective/rewards are explicit development policy, not original quest rules.
//! Later captures carry an extra maxed-out word absent from 7148; it is omitted.
use crate::{dat::Dat, script, world, world::WorldError};
use std::io;

pub const UPDATE: u32 = 0x82130024;
// Immutable authored profile. Changing rewards requires a new journal revision.
pub const TRAINING_QUEST: u32 = 0x4000ff01;
pub const TARGET_ENTITY: u32 = 0x470007d1;
pub const GOAL: u8 = 3;
pub const REWARD_XP: u64 = 120;
pub const REWARD_GOLD: u32 = 10;
pub fn rewards(command: &Command) -> Vec<crate::progression::Command> {
    if matches!(command, Command::Claim) {
        vec![
            crate::progression::Command::AwardXp(REWARD_XP),
            crate::progression::Command::AwardGold(REWARD_GOLD),
        ]
    } else {
        Vec::new()
    }
}
const ICON: u32 = 0x4100034b;
const UNDERWAY: u32 = 0x10000000;
const COMPLETED: u32 = 0x30000000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub killed: u8,
    pub claimed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    training: Option<Progress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Accept,
    Kill(u32),
    Claim,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestError {
    InvalidRecord,
    AlreadyAccepted,
    AlreadyCompleted,
    NotAccepted,
    NotReady,
}
impl std::fmt::Display for QuestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}
impl std::error::Error for QuestError {}
impl QuestError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::AlreadyAccepted => "You already accepted the training quest.",
            Self::AlreadyCompleted => "You already claimed this quest's reward.",
            Self::NotAccepted => "Accept the training quest first.",
            Self::NotReady => "Defeat three training creatures before claiming the reward.",
            Self::InvalidRecord => "Invalid development quest state.",
        }
    }
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Self::Accept => "a".into(),
            Self::Kill(entity) => format!("k,{entity:08x}"),
            Self::Claim => "c".into(),
        }
    }
    pub fn decode(text: &str) -> Result<Self, QuestError> {
        match text {
            "a" => Ok(Self::Accept),
            "c" => Ok(Self::Claim),
            _ => {
                let hex = text.strip_prefix("k,").ok_or(QuestError::InvalidRecord)?;
                if hex.len() != 8 || !hex.bytes().all(|value| value.is_ascii_hexdigit()) {
                    return Err(QuestError::InvalidRecord);
                }
                let entity = u32::from_str_radix(hex, 16).map_err(|_| QuestError::InvalidRecord)?;
                if entity >> 24 != 0x47 {
                    return Err(QuestError::InvalidRecord);
                }
                Ok(Self::Kill(entity))
            }
        }
    }
}

impl Book {
    pub fn get(&self) -> Option<&Progress> {
        self.training.as_ref()
    }
    pub fn ready(&self) -> bool {
        self.training
            .is_some_and(|state| state.killed == GOAL && !state.claimed)
    }
    pub fn apply(&self, command: &Command) -> Result<Self, QuestError> {
        if self
            .training
            .is_some_and(|state| state.killed > GOAL || state.claimed && state.killed != GOAL)
        {
            return Err(QuestError::InvalidRecord);
        }
        let mut next = self.clone();
        match command {
            Command::Accept => {
                if let Some(state) = self.training {
                    return Err(if state.claimed {
                        QuestError::AlreadyCompleted
                    } else {
                        QuestError::AlreadyAccepted
                    });
                }
                next.training = Some(Progress {
                    killed: 0,
                    claimed: false,
                });
            }
            Command::Kill(entity) => {
                next = self.record_kill(*entity);
            }
            Command::Claim => {
                let state = next.training.as_mut().ok_or(QuestError::NotAccepted)?;
                if state.claimed {
                    return Err(QuestError::AlreadyCompleted);
                }
                if state.killed != GOAL {
                    return Err(QuestError::NotReady);
                }
                state.claimed = true;
            }
        }
        Ok(next)
    }
    /// A qualifying kill is credited once by the authoritative encounter owner.
    /// The caller commits this candidate in the same transaction as kill rewards.
    pub fn record_kill(&self, entity: u32) -> Self {
        let mut next = self.clone();
        if entity == TARGET_ENTITY {
            if let Some(state) = &mut next.training {
                if !state.claimed && state.killed < GOAL {
                    state.killed += 1;
                }
            }
        }
        next
    }
    /// One live journal change. Never call for an unaccepted quest.
    pub fn update(&self) -> Result<Vec<u8>, WorldError> {
        let state = self.training.ok_or(WorldError::InvalidValue)?;
        let mut arguments = script::integer(0);
        arguments.extend(native_graph(0xa0, &info_payload(state, true)?));
        arguments.extend(script::integer(status(state)));
        script::private_call(UPDATE, &arguments)
    }
    /// The gmQuestInfoList argument used by UI_Load for saved journal restoration.
    pub fn list_graph(&self) -> Result<Vec<u8>, WorldError> {
        let mut payload = Vec::new();
        word(&mut payload, u32::from(self.training.is_some()));
        if let Some(state) = self.training {
            payload.extend(info_payload(state, false)?);
        }
        Ok(native_graph(0xa1, &payload))
    }
}

pub fn validate_assets(portal: &mut Dat) -> io::Result<()> {
    let icon = portal.read(ICON)?;
    if icon.get(..4) != Some(ICON.to_le_bytes().as_slice()) {
        return Err(io::Error::other(
            "quest icon has an incompatible DAT identity",
        ));
    }
    Ok(())
}

fn status(state: Progress) -> u32 {
    if state.claimed { COMPLETED } else { UNDERWAY }
}
fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}
fn native_graph(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [5, 1, 1, 0x44000100, 0, 0xffff0000 | u32::from(kind)] {
        word(&mut bytes, value);
    }
    bytes.extend(payload);
    word(&mut bytes, 0); // Native object has no interpreted heap fields.
    bytes
}
fn info_payload(state: Progress, play_fx: bool) -> Result<Vec<u8>, WorldError> {
    if state.killed > GOAL || state.claimed && state.killed != GOAL {
        return Err(WorldError::InvalidValue);
    }
    let mut bytes = Vec::new();
    word(&mut bytes, TRAINING_QUEST);
    bytes.extend(world::literal_name("Training Grounds")?);
    bytes.extend(world::literal_name(&format!(
        "Defeat three training creatures, then type @quest claim for {REWARD_XP} experience and {REWARD_GOLD} gold."
    ))?);
    word(&mut bytes, ICON);
    word(&mut bytes, status(state));
    word(&mut bytes, if state.claimed { 0 } else { 1 });
    let entry = if state.claimed {
        "Training complete. You claimed the reward.".into()
    } else if state.killed == GOAL {
        "Training creatures defeated: 3/3. Type @quest claim to receive your reward.".into()
    } else {
        format!("Training creatures defeated: {}/3.", state.killed)
    };
    bytes.extend(world::literal_name(&entry)?);
    word(&mut bytes, 1); // Authored challenge level.
    bytes.extend((-1.0f64).to_le_bytes()); // Unset bestowal timestamp, native Clear sentinel.
    bytes.extend((-1.0f64).to_le_bytes()); // Unset completion timestamp.
    word(&mut bytes, 0); // No expiration.
    bytes.extend(0f64.to_le_bytes());
    bytes.extend(0f64.to_le_bytes());
    word(&mut bytes, u32::from(play_fx));
    Ok(bytes)
}
