//! Server-owned progression and vital transitions. Client fields grant no rewards.
mod levels;
#[derive(Clone, Copy, Debug)]
pub struct Level {
    pub number: u16,
    pub xp: u64,
    pub health: u32,
    pub vigor: u32,
    pub credits: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progression {
    pub xp: u64,
    pub available_xp: u64,
    pub gold: u32,
    pub health: u32,
    pub vigor: u32,
}
impl Default for Progression {
    fn default() -> Self {
        Self {
            xp: 0,
            available_xp: 0,
            gold: 0,
            health: 100,
            vigor: 100,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Command {
    AwardXp(u64),
    SpendXp(u64),
    AwardGold(u32),
    SpendGold(u32),
    Damage(u32),
    Heal(u32),
    SpendVigor(u32),
    RecoverVigor(u32),
    Resurrect,
}
#[derive(Debug, Clone)]
pub enum ProgressionError {
    InvalidCommand,
    Overflow,
    InsufficientFunds,
    InsufficientVigor,
    InsufficientExperience,
    Dead,
    Alive,
}
impl std::fmt::Display for ProgressionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProgressionError {}
impl Progression {
    pub fn level(&self) -> &'static Level {
        &levels::LEVELS[levels::LEVELS
            .partition_point(|level| level.xp <= self.xp)
            .saturating_sub(1)]
    }
    pub fn alive(&self) -> bool {
        self.health > 0
    }
    pub fn valid(&self) -> bool {
        self.available_xp <= self.xp
            && self.xp <= i64::MAX as u64
            && self.gold <= i32::MAX as u32
            && self.health <= self.level().health
            && self.vigor <= self.level().vigor
    }
    pub fn apply(&self, command: &Command) -> Result<Self, ProgressionError> {
        if !self.valid() {
            return Err(ProgressionError::InvalidCommand);
        }
        let mut next = self.clone();
        match *command {
            Command::SpendXp(amount) => {
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                if !self.alive() {
                    return Err(ProgressionError::Dead);
                }
                next.available_xp = self
                    .available_xp
                    .checked_sub(amount)
                    .ok_or(ProgressionError::InsufficientExperience)?;
            }
            Command::AwardXp(amount) => {
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.xp = self
                    .xp
                    .checked_add(amount)
                    .filter(|xp| *xp <= i64::MAX as u64)
                    .ok_or(ProgressionError::Overflow)?;
                next.available_xp = self
                    .available_xp
                    .checked_add(amount)
                    .filter(|xp| *xp <= i64::MAX as u64)
                    .ok_or(ProgressionError::Overflow)?;
                // Local profile preserves missing vital amounts on level-up;
                // gaining a level must not resurrect a dead character.
                if self.alive() {
                    next.health = self.health + next.level().health - self.level().health;
                    next.vigor = self.vigor + next.level().vigor - self.level().vigor;
                }
            }
            Command::AwardGold(amount) => {
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.gold = self
                    .gold
                    .checked_add(amount)
                    .filter(|gold| *gold <= i32::MAX as u32)
                    .ok_or(ProgressionError::Overflow)?;
            }
            Command::SpendGold(amount) => {
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.gold = self
                    .gold
                    .checked_sub(amount)
                    .ok_or(ProgressionError::InsufficientFunds)?;
            }
            Command::Damage(amount) => {
                if !self.alive() {
                    return Err(ProgressionError::Dead);
                }
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.health = self.health.saturating_sub(amount);
            }
            Command::Heal(amount) => {
                if !self.alive() {
                    return Err(ProgressionError::Dead);
                }
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.health = self.health.saturating_add(amount).min(self.level().health);
            }
            Command::SpendVigor(amount) => {
                if !self.alive() {
                    return Err(ProgressionError::Dead);
                }
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.vigor = self
                    .vigor
                    .checked_sub(amount)
                    .ok_or(ProgressionError::InsufficientVigor)?;
            }
            Command::RecoverVigor(amount) => {
                if !self.alive() {
                    return Err(ProgressionError::Dead);
                }
                if amount == 0 {
                    return Err(ProgressionError::InvalidCommand);
                }
                next.vigor = self.vigor.saturating_add(amount).min(self.level().vigor);
            }
            Command::Resurrect => {
                if self.alive() {
                    return Err(ProgressionError::Alive);
                }
                next.health = self.level().health;
                next.vigor = self.level().vigor;
            }
        }
        Ok(next)
    }
    pub fn int_stats(&self) -> [(u32, u32); 8] {
        let level = self.level();
        [
            (0x100, level.health),
            (0x101, level.vigor),
            (0x102, self.health),
            (0x103, self.vigor),
            (0x107, level.health),
            (0x108, level.vigor),
            (0x12e, u32::from(level.number)),
            (0xc1c, self.gold),
        ]
    }
    pub fn private_updates(&self) -> Vec<Vec<u8>> {
        self.private_since(None)
    }
    /// Ordered deltas against the preceding committed state, not a guessed cache.
    pub fn private_changes(&self, previous: &Self) -> Vec<Vec<u8>> {
        self.private_since(Some(previous))
    }
    fn private_since(&self, previous: Option<&Self>) -> Vec<Vec<u8>> {
        let old_stats = previous.map(Self::int_stats);
        let mut events: Vec<_> = self
            .int_stats()
            .into_iter()
            .enumerate()
            .filter(|(index, (_, value))| old_stats.is_none_or(|old| old[*index].1 != *value))
            .map(|(_, (key, value))| {
                [
                    0x10069u32.to_le_bytes(),
                    key.to_le_bytes(),
                    value.to_le_bytes(),
                ]
                .concat()
            })
            .collect();
        for (key, value, old) in [
            (0x12cu32, self.xp, previous.map(|state| state.xp)),
            (
                0x12d,
                self.available_xp,
                previous.map(|state| state.available_xp),
            ),
        ] {
            if old == Some(value) {
                continue;
            }
            let mut event = 0x100d4u32.to_le_bytes().to_vec();
            event.extend(key.to_le_bytes());
            event.extend(value.to_le_bytes());
            events.push(event);
        }
        if previous.is_none_or(|old| old.alive() != self.alive()) {
            events.push(
                [
                    0x1006bu32.to_le_bytes(),
                    0x117u32.to_le_bytes(),
                    u32::from(!self.alive()).to_le_bytes(),
                ]
                .concat(),
            );
        }
        events
    }
}
impl Command {
    pub fn encode(&self) -> String {
        match self {
            Self::AwardXp(v) => format!("x,{v}"),
            Self::SpendXp(v) => format!("a,{v}"),
            Self::AwardGold(v) => format!("g,{v}"),
            Self::SpendGold(v) => format!("s,{v}"),
            Self::Damage(v) => format!("d,{v}"),
            Self::Heal(v) => format!("h,{v}"),
            Self::SpendVigor(v) => format!("v,{v}"),
            Self::RecoverVigor(v) => format!("r,{v}"),
            Self::Resurrect => "z".into(),
        }
    }
    pub fn decode(text: &str) -> Result<Self, ProgressionError> {
        if text == "z" {
            return Ok(Self::Resurrect);
        }
        if text.len() > 32 {
            return Err(ProgressionError::InvalidCommand);
        }
        let (kind, value) = text
            .split_once(',')
            .ok_or(ProgressionError::InvalidCommand)?;
        let value = value
            .parse::<u64>()
            .map_err(|_| ProgressionError::InvalidCommand)?;
        if kind == "x" {
            return Ok(Self::AwardXp(value));
        }
        if kind == "a" {
            return Ok(Self::SpendXp(value));
        }
        let value = u32::try_from(value).map_err(|_| ProgressionError::InvalidCommand)?;
        Ok(match kind {
            "g" => Self::AwardGold(value),
            "s" => Self::SpendGold(value),
            "d" => Self::Damage(value),
            "h" => Self::Heal(value),
            "v" => Self::SpendVigor(value),
            "r" => Self::RecoverVigor(value),
            _ => return Err(ProgressionError::InvalidCommand),
        })
    }
}
