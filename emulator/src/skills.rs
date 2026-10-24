//! Persistent skill eligibility and native train/untrain routing.
//! Advancement curves and active effects are not inferred from metadata alone.
use crate::{dat::Dat, progression::Progression};
use std::{collections::BTreeMap, sync::OnceLock};
mod advancement;
pub const TRAIN: u32 = 0x82140002;
pub const UNTRAIN: u32 = 0x82140007;
pub const RAISE: u32 = 0x8214000c;
pub const USE: u32 = 0x8204000b;
pub const LIFESTONE_RECALL: u32 = 0x343;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Train,
    Untrain,
    Raise,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trained {
    pub rank: u16,
    pub credits: u16,
    pub locked: bool,
    pub xp: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    entries: BTreeMap<u32, Trained>,
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Train(u32),
    Grant(u32),
    Untrain(u32),
}
#[derive(Clone, Debug)]
pub enum SkillError {
    Catalog,
    Unknown,
    UnsupportedProfile,
    Prerequisite,
    Incompatible,
    InsufficientCredits,
    AlreadyTrained,
    NotTrained,
    Locked,
    Dependents,
    Capacity,
    Dead,
    AdvancementUnavailable,
    InsufficientXp,
    MaximumRank,
}
impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SkillError {}
impl SkillError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Prerequisite => "You do not meet this skill's requirements.",
            Self::Incompatible => "This skill conflicts with a skill you already know.",
            Self::InsufficientCredits => "You do not have enough skill credits.",
            Self::AlreadyTrained => "You already know this skill.",
            Self::NotTrained => "You have not trained this skill.",
            Self::Locked => "This skill cannot be untrained.",
            Self::Dependents => "Another trained skill depends on this skill.",
            Self::Dead => "You cannot change skills while dead.",
            Self::AdvancementUnavailable => "Skill advancement is not available yet.",
            Self::InsufficientXp => "You do not have enough available experience.",
            Self::MaximumRank => "This skill is at its supported maximum level.",
            Self::Capacity => "You cannot train another skill.",
            _ => "This skill is not available for this character.",
        }
    }
}
struct Definition {
    record: u32,
    fingerprint: u64,
    race: u32,
    level: u16,
    credits: u16,
    cap: u16,
    kind: u32,
    curve: u32,
    required: BTreeMap<u32, u16>,
    forbidden: BTreeMap<u32, u16>,
}
struct Catalog {
    index_fingerprint: u64,
    definitions: BTreeMap<u32, Definition>,
}
fn fingerprint(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |hash, value| {
        (hash ^ u64::from(*value)).wrapping_mul(0x100000001b3)
    })
}
fn catalog() -> Result<&'static Catalog, SkillError> {
    static CATALOG: OnceLock<Result<Catalog, SkillError>> = OnceLock::new();
    CATALOG
        .get_or_init(|| Catalog::decode(include_str!("../content/skills.tsv")))
        .as_ref()
        .map_err(Clone::clone)
}
impl Catalog {
    fn decode(text: &str) -> Result<Self, SkillError> {
        let mut lines = text.lines();
        if lines.next() != Some("pyrealium-skills-v1") {
            return Err(SkillError::Catalog);
        }
        let mut definitions = BTreeMap::new();
        let mut index_fingerprint = None;
        let dependencies = |text: &str| -> Result<BTreeMap<u32, u16>, SkillError> {
            let mut values = BTreeMap::new();
            if text == "-" {
                return Ok(values);
            }
            for entry in text.split(',') {
                let (key, rank) = entry.split_once(':').ok_or(SkillError::Catalog)?;
                let key = u32::from_str_radix(key, 16).map_err(|_| SkillError::Catalog)?;
                let rank = rank.parse::<u16>().map_err(|_| SkillError::Catalog)?;
                if values.len() >= 256
                    || key == 0
                    || key > 65535
                    || rank > 10000
                    || values.insert(key, rank).is_some()
                {
                    return Err(SkillError::Catalog);
                }
            }
            Ok(values)
        };
        for line in lines {
            if let Some(value) = line.strip_prefix("# Index FNV64: ") {
                if index_fingerprint
                    .replace(u64::from_str_radix(value, 16).map_err(|_| SkillError::Catalog)?)
                    .is_some()
                {
                    return Err(SkillError::Catalog);
                }
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<_> = line.split('\t').collect();
            if f.len() != 12 || line.len() > 8192 {
                return Err(SkillError::Catalog);
            }
            let hex = |i: usize| u32::from_str_radix(f[i], 16).map_err(|_| SkillError::Catalog);
            let id = hex(0)?;
            let definition = Definition {
                record: hex(1)?,
                fingerprint: u64::from_str_radix(f[3], 16).map_err(|_| SkillError::Catalog)?,
                race: f[4].parse().map_err(|_| SkillError::Catalog)?,
                level: f[5].parse().map_err(|_| SkillError::Catalog)?,
                credits: f[6].parse().map_err(|_| SkillError::Catalog)?,
                cap: f[7].parse().map_err(|_| SkillError::Catalog)?,
                kind: f[9].parse().map_err(|_| SkillError::Catalog)?,
                curve: hex(8)?,
                required: dependencies(f[10])?,
                forbidden: dependencies(f[11])?,
            };
            if id == 0
                || id > 65535
                || definition.record >> 24 != 0x70
                || hex(8)? >> 24 != 0x6f
                || definition.race > 7
                || definition.level == 0
                || definition.level > 150
                || definition.credits > 1000
                || definition.cap == 0
                || definition.cap > 10000
                || definitions.len() >= 8192
                || definitions.insert(id, definition).is_some()
            {
                return Err(SkillError::Catalog);
            }
        }
        Ok(Self {
            index_fingerprint: index_fingerprint.ok_or(SkillError::Catalog)?,
            definitions,
        })
    }
}
pub fn validate_assets(portal: &mut Dat) -> std::io::Result<()> {
    advancement::validate(portal)?;
    let catalog = catalog().map_err(std::io::Error::other)?;
    if fingerprint(&portal.read(0x700016e2)?) != catalog.index_fingerprint {
        return Err(std::io::Error::other(
            "skill index differs from imported metadata",
        ));
    }
    for definition in catalog.definitions.values() {
        if fingerprint(&portal.read(definition.record)?) != definition.fingerprint {
            return Err(std::io::Error::other(format!(
                "skill definition changed: {:#010x}",
                definition.record
            )));
        }
    }
    Ok(())
}
impl Book {
    /// Explicit development binding grant, separate from original entitlement.
    pub fn with_lifestone_recall(
        &self,
        state: &Progression,
        race: u32,
    ) -> Result<Self, SkillError> {
        if !state.alive() {
            return Err(SkillError::Dead);
        }
        let definition = catalog()?
            .definitions
            .get(&LIFESTONE_RECALL)
            .ok_or(SkillError::Unknown)?;
        if race != 1
            || definition.race != 0
            || definition.level != 1
            || definition.cap != 1
            || definition.credits != 0
            || definition.kind != 0x44
            || !definition.required.is_empty()
            || !definition.forbidden.is_empty()
        {
            return Err(SkillError::UnsupportedProfile);
        }
        if let Some(entry) = self.entries.get(&LIFESTONE_RECALL) {
            if !entry.locked || entry.credits != 0 || entry.xp != 0 || entry.rank != 1 {
                return Err(SkillError::Catalog);
            }
            return Ok(self.clone());
        }
        if self.entries.len() >= 128 {
            return Err(SkillError::Capacity);
        }
        let mut next = self.clone();
        next.entries.insert(
            LIFESTONE_RECALL,
            Trained {
                rank: 1,
                credits: 0,
                locked: true,
                xp: 0,
            },
        );
        Ok(next)
    }
    pub fn advance(
        &self,
        id: u32,
        state: &Progression,
        race: u32,
    ) -> Result<(Self, Progression), SkillError> {
        if !state.alive() {
            return Err(SkillError::Dead);
        }
        let known = self.entries.get(&id).ok_or(SkillError::NotTrained)?;
        let definition = catalog()?.definitions.get(&id).ok_or(SkillError::Unknown)?;
        if race != 1
            || definition.race != 1
            || definition.level > 50
            || matches!(definition.kind, 4 | 0x44 | 0x45 | 0x51 | 0x71)
        {
            return Err(SkillError::UnsupportedProfile);
        }
        let (rank, total) = advancement::get(definition.curve)?.next(known.xp, definition.cap)?;
        let cost = total
            .checked_sub(known.xp)
            .filter(|cost| *cost > 0)
            .ok_or(SkillError::Catalog)?;
        if cost > state.available_xp {
            return Err(SkillError::InsufficientXp);
        }
        let progression = state
            .apply(&crate::progression::Command::SpendXp(cost))
            .map_err(|_| SkillError::Catalog)?;
        let mut next = self.clone();
        let entry = next.entries.get_mut(&id).unwrap();
        entry.rank = rank;
        entry.xp = total;
        Ok((next, progression))
    }
    pub fn get(&self, id: u32) -> Option<&Trained> {
        self.entries.get(&id)
    }
    pub fn entries(&self) -> impl Iterator<Item = (u32, &Trained)> {
        self.entries.iter().map(|(&id, entry)| (id, entry))
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn available_credits(&self, state: &Progression) -> u16 {
        u32::from(state.level().credits).saturating_sub(
            self.entries
                .values()
                .map(|entry| u32::from(entry.credits))
                .sum(),
        ) as u16
    }
    pub fn apply(
        &self,
        command: Command,
        state: &Progression,
        race: u32,
    ) -> Result<Self, SkillError> {
        if !state.alive() {
            return Err(SkillError::Dead);
        }
        let catalog = catalog()?;
        let mut next = self.clone();
        match command {
            Command::Train(id) | Command::Grant(id) => {
                let definition = catalog.definitions.get(&id).ok_or(SkillError::Unknown)?;
                let granting = matches!(command, Command::Grant(_));
                // Current native character profile supports human training only.
                // Faction/hero/recall/mob grants require additional state/services.
                if race != 1
                    || definition.race != 1
                    || definition.level > 50
                    || matches!(definition.kind, 4 | 0x44 | 0x45 | 0x51 | 0x71)
                    || granting && definition.credits != 0
                    || !granting && definition.credits == 0
                {
                    return Err(SkillError::UnsupportedProfile);
                }
                if self.entries.contains_key(&id) {
                    return Err(SkillError::AlreadyTrained);
                }
                if self.entries.len() >= 128 {
                    return Err(SkillError::Capacity);
                }
                if state.level().number < definition.level
                    || definition.required.iter().any(|(id, rank)| {
                        !self
                            .entries
                            .get(id)
                            .is_some_and(|entry| entry.rank >= *rank)
                    })
                {
                    return Err(SkillError::Prerequisite);
                }
                if definition
                    .forbidden
                    .keys()
                    .any(|id| self.entries.contains_key(id))
                    || self.entries.keys().any(|known| {
                        catalog
                            .definitions
                            .get(known)
                            .is_some_and(|known| known.forbidden.contains_key(&id))
                    })
                {
                    return Err(SkillError::Incompatible);
                }
                if self.available_credits(state) < definition.credits {
                    return Err(SkillError::InsufficientCredits);
                }
                next.entries.insert(
                    id,
                    Trained {
                        rank: 0,
                        credits: definition.credits,
                        locked: granting,
                        xp: 0,
                    },
                );
            }
            Command::Untrain(id) => {
                let known = self.entries.get(&id).ok_or(SkillError::NotTrained)?;
                if known.locked {
                    return Err(SkillError::Locked);
                }
                if self.entries.keys().any(|other| {
                    *other != id
                        && catalog
                            .definitions
                            .get(other)
                            .is_some_and(|definition| definition.required.contains_key(&id))
                }) {
                    return Err(SkillError::Dependents);
                }
                // Immediate rank-zero untraining is the supported local policy.
                // XP advancement/timed untraining must not be guessed here.
                if known.rank != 0 {
                    return Err(SkillError::AdvancementUnavailable);
                }
                next.entries.remove(&id);
            }
        }
        Ok(next)
    }
}
impl Command {
    pub fn encode(self) -> String {
        match self {
            Self::Train(id) => format!("t,{id:x}"),
            Self::Grant(id) => format!("g,{id:x}"),
            Self::Untrain(id) => format!("u,{id:x}"),
        }
    }
    pub fn decode(text: &str) -> Result<Self, SkillError> {
        if text.len() > 16 {
            return Err(SkillError::Catalog);
        }
        let (kind, id) = text.split_once(',').ok_or(SkillError::Catalog)?;
        let id = u32::from_str_radix(id, 16).map_err(|_| SkillError::Catalog)?;
        if id == 0 || id > 65535 {
            return Err(SkillError::Unknown);
        }
        Ok(match kind {
            "t" => Self::Train(id),
            "g" => Self::Grant(id),
            "u" => Self::Untrain(id),
            _ => return Err(SkillError::Catalog),
        })
    }
}
