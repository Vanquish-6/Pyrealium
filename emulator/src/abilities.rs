//! Bounded active-ability execution for the explicit development player profile.
//!
//! Skill 0x55 is identified by the pinned native catalog: human, level 3,
//! one training credit, cap 50, prerequisite 0x59. Its healing amount, vigor
//! cost and cooldown below are authored development policy, not recovered
//! original combat balance. No client argument determines a cost or reward.
//!
//! A prepared cast borrows the runtime exclusively. Admit its UI batch, persist
//! `commands()` atomically, then call `commit()`. Dropping it on any earlier
//! error spends no cooldown. The consuming commit cannot fail after persistence.
use crate::{
    progression::{Command, Progression},
    skills::Book,
};
use std::{collections::BTreeMap, fmt};

pub const EASE_SUFFERING: u32 = 0x55;
pub const HEAL_AMOUNT: u32 = 30;
pub const VIGOR_COST: u32 = 10;
pub const COOLDOWN_MS: u64 = 8_000;
pub const GLOBAL_COOLDOWN_MS: u64 = 1_000;
const MAX_SKILL_COOLDOWNS: usize = 256;
const MAX_PLAYER_COOLDOWNS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbilityError {
    Catalog,
    Unsupported,
    InvalidCharacter,
    NotLearned,
    UnsupportedCharacter,
    InvalidTarget,
    Dead,
    FullHealth,
    InsufficientVigor,
    Cooldown { remaining_ms: u64 },
    GlobalCooldown { remaining_ms: u64 },
    Capacity,
    Clock,
    InvalidProgression,
}
impl fmt::Display for AbilityError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog => out.write_str("The development healing profile does not match the skill catalog."),
            Self::Unsupported => out.write_str("This active skill is not supported by the current ability profile."),
            Self::InvalidCharacter => out.write_str("The active character is unavailable."),
            Self::NotLearned => out.write_str("You have not learned this skill."),
            Self::UnsupportedCharacter => out.write_str("This skill requires the supported human character and level."),
            Self::InvalidTarget => out.write_str("Development healing currently supports yourself only. Clear your target or select yourself."),
            Self::Dead => out.write_str("You cannot use this skill while dead."),
            Self::FullHealth => out.write_str("Your health is already full."),
            Self::InsufficientVigor => out.write_str("You do not have enough vigor for this skill."),
            Self::Cooldown { remaining_ms } => write!(out,"This skill is recovering. Wait {} second(s).",remaining_ms.div_ceil(1_000)),
            Self::GlobalCooldown { remaining_ms } => write!(out,"Wait {} second(s) before using another development ability.",remaining_ms.div_ceil(1_000)),
            Self::Capacity => out.write_str("The ability service is at capacity. Try again shortly."),
            Self::Clock => out.write_str("The ability clock is unavailable."),
            Self::InvalidProgression => out.write_str("The current character vitals cannot accept this skill."),
        }
    }
}
impl std::error::Error for AbilityError {}

pub struct Runtime {
    skills: BTreeMap<(u64, u32), u64>,
    players: BTreeMap<u64, u64>,
    last_now: u64,
}
impl Runtime {
    /// Caller must enable this only as an explicit authored development profile.
    pub fn development() -> Result<Self, AbilityError> {
        let mut matching = include_str!("../content/skills.tsv")
            .lines()
            .filter(|line| line.split('\t').next() == Some("55"));
        let record = matching.next().ok_or(AbilityError::Catalog)?;
        if matching.next().is_some() {
            return Err(AbilityError::Catalog);
        }
        let fields: Vec<_> = record.split('\t').collect();
        if fields.len() != 12
            || fields[1] != "70001729"
            || fields[2] != "96e453fa8275af8d7ba10909b2e9fb83a036816c1cef0d6e70413c031779af45"
            || fields[3] != "c20d6f529add398f"
            || fields[4..] != ["1", "3", "1", "50", "6f001ef5", "65", "59:0", "-"]
        {
            return Err(AbilityError::Catalog);
        }
        Ok(Self {
            skills: BTreeMap::new(),
            players: BTreeMap::new(),
            last_now: 0,
        })
    }

    pub fn supports(&self, skill: u32) -> bool {
        skill == EASE_SUFFERING
    }

    /// Expiry is safe during failed casts and disconnects. Never erase an
    /// unexpired entry when the same character logs out or changes targets.
    pub fn expire(&mut self, now: u64) -> Result<(), AbilityError> {
        if now < self.last_now {
            return Err(AbilityError::Clock);
        }
        self.last_now = now;
        self.skills.retain(|_, until| *until > now);
        self.players.retain(|_, until| *until > now);
        Ok(())
    }

    /// Only self/no-target healing is currently supported. The native target
    /// sentinels 0 and u64::MAX both mean no selected target in this server.
    pub fn prepare<'a>(
        &'a mut self,
        player: u64,
        skill: u32,
        target: u64,
        race: u32,
        book: &Book,
        state: &Progression,
        now: u64,
    ) -> Result<PreparedCast<'a>, AbilityError> {
        if !self.supports(skill) {
            return Err(AbilityError::Unsupported);
        }
        if player == 0 || player == u64::MAX {
            return Err(AbilityError::InvalidCharacter);
        }
        let learned = book.get(skill).ok_or(AbilityError::NotLearned)?;
        if race != 1 || state.level().number < 3 || learned.rank > 50 {
            return Err(AbilityError::UnsupportedCharacter);
        }
        if ![0, u64::MAX, player].contains(&target) {
            return Err(AbilityError::InvalidTarget);
        }
        if !state.valid() {
            return Err(AbilityError::InvalidProgression);
        }
        if !state.alive() {
            return Err(AbilityError::Dead);
        }
        if state.health >= state.level().health {
            return Err(AbilityError::FullHealth);
        }
        if state.vigor < VIGOR_COST {
            return Err(AbilityError::InsufficientVigor);
        }
        self.expire(now)?;
        if let Some(&until) = self.skills.get(&(player, skill)) {
            return Err(AbilityError::Cooldown {
                remaining_ms: until - now,
            });
        }
        if let Some(&until) = self.players.get(&player) {
            return Err(AbilityError::GlobalCooldown {
                remaining_ms: until - now,
            });
        }
        if self.skills.len() >= MAX_SKILL_COOLDOWNS || self.players.len() >= MAX_PLAYER_COOLDOWNS {
            return Err(AbilityError::Capacity);
        }
        let until = now.checked_add(COOLDOWN_MS).ok_or(AbilityError::Clock)?;
        let global_until = now
            .checked_add(GLOBAL_COOLDOWN_MS)
            .ok_or(AbilityError::Clock)?;
        let commands = [Command::SpendVigor(VIGOR_COST), Command::Heal(HEAL_AMOUNT)];
        let next = commands.iter().try_fold(state.clone(), |value, command| {
            value
                .apply(command)
                .map_err(|_| AbilityError::InvalidProgression)
        })?;
        Ok(PreparedCast {
            runtime: self,
            player,
            skill,
            until,
            global_until,
            commands,
            previous: state.clone(),
            next,
        })
    }
}

/// Not Clone: a prepared cast can spend its validated cooldown exactly once.
/// Its mutable runtime borrow prevents another cast from invalidating capacity
/// or cooldown checks while the caller admits packets and persists progression.
pub struct PreparedCast<'a> {
    runtime: &'a mut Runtime,
    player: u64,
    skill: u32,
    until: u64,
    global_until: u64,
    commands: [Command; 2],
    previous: Progression,
    next: Progression,
}
impl PreparedCast<'_> {
    pub fn player(&self) -> u64 {
        self.player
    }
    pub fn skill(&self) -> u32 {
        self.skill
    }
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
    pub fn previous(&self) -> &Progression {
        &self.previous
    }
    pub fn next(&self) -> &Progression {
        &self.next
    }
    pub fn healed(&self) -> u32 {
        self.next.health - self.previous.health
    }
    pub fn ready_at(&self) -> u64 {
        self.until
    }
    /// Call only after the atomic progression batch has been durably saved.
    pub fn commit(self) {
        self.runtime
            .skills
            .insert((self.player, self.skill), self.until);
        self.runtime.players.insert(self.player, self.global_until);
    }
}
