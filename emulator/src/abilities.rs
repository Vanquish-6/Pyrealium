//! Active skill use driven by portal.dat skill records (`content/skill-effects.tsv`
//! and `content/skill-travel.tsv`).
//!
//! Each record supplies reset time, vigor cost, max range, valid targets, hand
//! implement requirements, story behavior and FX, an attack damage table and its
//! effects, all valued at the player's trained rank by linear interpolation
//! between the record's level points. Values match archived retail skill pages
//! (Ease Suffering: 100 vigor, 40 m, 7 s, heal 25 at L1 to 250 at L50; Flame
//! Bolt: stave, 32 vigor, 60 m, 3 to 75 plus weapon damage, burn 1 to 4 per
//! second for 30 s).
//!
//! Applied effects: instant and over-time health (0x21, 0x11) and vigor (0x22,
//! 0x12), health and vigor regeneration multipliers (stats 0x30106, 0x30107),
//! attack damage added to weapon damage, weapon-only attacks, travel to the
//! lifestone, the last portal, or a recall telepad, and timed armor, attack and
//! defense ratings. Armor uses NaturalArmor 0x40125 and ArmorLevel 0x40262.
//! Attack ratings are 0x800001/7/c and defense ratings 0x800002/8/d (magic,
//! melee, missile). Other buff stats, projectile flight, area attacks, combos
//! and summoned portals are not applied.
//!
//! A prepared cast borrows the runtime exclusively. Admit its UI batch, persist
//! `commands()` atomically, then call `commit()`. Dropping it on any earlier
//! error spends no cooldown. The consuming commit cannot fail after persistence.
use crate::{
    encounters::{Over, Regen},
    movement::Placement,
    progression::{Command, Progression},
    skills::Book,
};
use std::{collections::BTreeMap, fmt};

pub const GLOBAL_COOLDOWN_MS: u64 = 1_000;
/// Retail skill pages list a minimum vigor of 1; the cost then drains what remains.
pub const MIN_VIGOR: u32 = 1;
const MAX_SKILL_COOLDOWNS: usize = 256;
const MAX_PLAYER_COOLDOWNS: usize = 128;
const SELF: u32 = 0x1;
const MONSTER: u32 = 0x80;
const PLAYER: u32 = 0x100;
const HEALTH: u32 = 0x21;
const VIGOR: u32 = 0x22;
const HEALTH_OVER_TIME: u32 = 0x11;
const VIGOR_OVER_TIME: u32 = 0x12;
const HEALTH_REGEN: u32 = 0x30106;
const VIGOR_REGEN: u32 = 0x30107;
const NATURAL_ARMOR: u32 = 0x40125;
const ARMOR_LEVEL: u32 = 0x40262;
/// Magic, melee, missile. Matches SkillList attack 1/7/c and defense 2/8/d.
const ATTACK_RATING: [u32; 3] = [0x800001, 0x800007, 0x80000c];
const DEFENSE_RATING: [u32; 3] = [0x800002, 0x800008, 0x80000d];
/// Weapon-attack effect type carried by the racial basic attacks.
const WEAPON_ATTACK: u32 = 0x6;
const BASIC_ATTACK_KIND: u32 = 0x44;
const ANY_AND_NONE: u32 = 0xffff_ffff;
const NO_IMPLEMENT: u32 = 0x1;
/// Implement type names (enum 23000003) used in requirement messages.
const IMPLEMENTS: [(u32, &str); 20] = [
    (0x1, "an empty hand"),
    (0x2, "a sword"),
    (0x4, "a bow"),
    (0x8, "a drum"),
    (0x10, "a shield"),
    (0x20, "an axe"),
    (0x40, "a dagger"),
    (0x100, "a hammer"),
    (0x400, "a two-handed sword"),
    (0x800, "an alchemy kit"),
    (0x8000, "a spear"),
    (0x10000, "a jailai"),
    (0x20000, "a flail"),
    (0x40000, "a stave"),
    (0x80000, "a scythe"),
    (0x80062, "a one-handed weapon"),
    (0x100000, "a hand weapon"),
    (0x200000, "an orb"),
    (0x3fffee, "a weapon"),
    (0xfffffffe, "an item"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbilityError {
    Catalog,
    Unsupported,
    InvalidCharacter,
    NotLearned,
    InvalidTarget,
    TargetCreature,
    OutOfRange,
    Hand { right: bool, required: u32 },
    Dead,
    NothingToRestore,
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
            Self::Catalog => out.write_str("The skill effect table is invalid."),
            Self::Unsupported => out.write_str("This skill's effects are not supported yet."),
            Self::InvalidCharacter => out.write_str("The active character is unavailable."),
            Self::NotLearned => out.write_str("You have not learned this skill."),
            Self::InvalidTarget => out.write_str("That is not a valid target for this skill."),
            Self::TargetCreature => out.write_str("Select a creature to use this skill."),
            Self::OutOfRange => out.write_str("Your target is too far away for this skill."),
            Self::Hand { right, required } => write!(
                out,
                "This skill requires {} in your {} hand.",
                implement_name(*required),
                if *right { "right" } else { "left" }
            ),
            Self::Dead => out.write_str("You cannot use this skill while dead."),
            Self::NothingToRestore => out.write_str("Your health and vigor are already full."),
            Self::InsufficientVigor => out.write_str("You do not have enough vigor for this skill."),
            Self::Cooldown { remaining_ms } => write!(out, "This skill is recovering. Wait {} second(s).", remaining_ms.div_ceil(1_000)),
            Self::GlobalCooldown { remaining_ms } => write!(out, "Wait {} second(s) before using another skill.", remaining_ms.div_ceil(1_000)),
            Self::Capacity => out.write_str("The skill service is at capacity. Try again shortly."),
            Self::Clock => out.write_str("The skill clock is unavailable."),
            Self::InvalidProgression => out.write_str("The current character vitals cannot accept this skill."),
        }
    }
}
impl std::error::Error for AbilityError {}

fn implement_name(mask: u32) -> &'static str {
    IMPLEMENTS
        .iter()
        .find(|(bits, _)| *bits == mask)
        .or_else(|| IMPLEMENTS.iter().find(|(bits, _)| mask & bits != 0 && bits.count_ones() == 1))
        .map_or("a different implement", |(_, name)| name)
}

/// Level points (level, value), sorted by level.
#[derive(Clone, Debug, Default, PartialEq)]
struct Table(Vec<(f32, f32)>);
impl Table {
    fn parse(text: &str) -> Result<Self, AbilityError> {
        if text == "-" {
            return Ok(Self::default());
        }
        let mut points = Vec::new();
        for pair in text.split(',') {
            let (level, value) = pair.split_once(':').ok_or(AbilityError::Catalog)?;
            let level: f32 = level.parse().map_err(|_| AbilityError::Catalog)?;
            let value: f32 = value.parse().map_err(|_| AbilityError::Catalog)?;
            if !level.is_finite() || !value.is_finite() || points.len() >= 64 {
                return Err(AbilityError::Catalog);
            }
            points.push((level, value));
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self(points))
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Linear between points, flat outside them.
    fn at(&self, rank: u16) -> f32 {
        let rank = f32::from(rank.max(1));
        let points = &self.0;
        let Some(&(first_level, first_value)) = points.first() else {
            return 0.0;
        };
        if rank <= first_level {
            return first_value;
        }
        for window in points.windows(2) {
            let ((low, low_value), (high, high_value)) = (window[0], window[1]);
            if rank <= high {
                let span = (high - low).max(f32::EPSILON);
                return low_value + (high_value - low_value) * (rank - low) / span;
            }
        }
        points.last().unwrap().1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Target,
    Own,
}

#[derive(Clone, Debug)]
struct Timed {
    side: Side,
    vigor: bool,
    values: Table,
    period_ms: u64,
    duration: Table,
}

#[derive(Clone, Debug)]
struct Buff {
    side: Side,
    health: Table,
    vigor: Table,
    duration: Table,
}
#[derive(Clone, Debug)]
struct RatingSpec {
    side: Side,
    duration: Table,
    armor: Vec<Table>,
    attack: [Vec<Table>; 3],
    defense: [Vec<Table>; 3],
}
impl RatingSpec {
    fn useful(&self) -> bool {
        !self.armor.is_empty()
            || self.attack.iter().any(|tables| !tables.is_empty())
            || self.defense.iter().any(|tables| !tables.is_empty())
    }
    fn resolve(&self, rank: u16) -> (crate::combat::Ratings, u64) {
        let sum = |tables: &[Table]| -> i32 { tables.iter().map(|table| table.at(rank).round() as i32).sum() };
        let ratings = crate::combat::Ratings {
            armor: sum(&self.armor),
            attack: std::array::from_fn(|index| sum(&self.attack[index])),
            defense: std::array::from_fn(|index| sum(&self.defense[index])),
        };
        let seconds = self.duration.at(rank).max(0.0);
        let duration_ms = if seconds <= 0.0 { 180_000 } else { (seconds * 1000.0) as u64 };
        (ratings, duration_ms)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Travel {
    Lifestone,
    Portal,
    Telepads(Vec<Placement>),
}

#[derive(Clone, Debug)]
struct Definition {
    kind: u32,
    reset_ms: u64,
    vigor: u32,
    range: f32,
    targets: u32,
    right: u32,
    left: u32,
    story: u32,
    fx: u32,
    damage: Table,
    health: Table,
    vigor_effect: Table,
    timed: Vec<Timed>,
    buff: Option<Buff>,
    ratings: Vec<RatingSpec>,
    weapon: bool,
    travel: Option<Travel>,
}
impl Definition {
    fn harmful(&self) -> bool {
        self.weapon
            || !self.damage.is_empty()
            || self.health.0.iter().any(|&(_, value)| value < 0.0)
            || self
                .timed
                .iter()
                .any(|t| t.side == Side::Target && t.values.0.iter().any(|&(_, v)| v < 0.0))
    }
    fn usable(&self) -> bool {
        self.weapon
            || self.travel.is_some()
            || !self.damage.is_empty()
            || !self.health.is_empty()
            || !self.vigor_effect.is_empty()
            || !self.timed.is_empty()
            || self.buff.is_some()
            || self.ratings.iter().any(RatingSpec::useful)
    }
    fn ratings_for(&self, side: Side, rank: u16) -> Option<(crate::combat::Ratings, u64)> {
        let mut total = crate::combat::Ratings::default();
        let mut duration = 0u64;
        let mut found = false;
        for spec in &self.ratings {
            if spec.side != side || !spec.useful() {
                continue;
            }
            let (part, ms) = spec.resolve(rank);
            total.add(part);
            duration = duration.max(ms);
            found = true;
        }
        (found && !total.is_empty()).then_some((total, duration))
    }
}

/// What the caster holds. `Unknown` items have no implement metadata and pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Empty,
    Implement(u32),
    Unknown,
}
fn holds(required: u32, hand: Hand) -> bool {
    match hand {
        _ if required == ANY_AND_NONE => true,
        Hand::Unknown => true,
        Hand::Empty => required & NO_IMPLEMENT != 0,
        Hand::Implement(bits) => required & bits != 0,
    }
}

/// Where the player aimed the skill. `Player` carries the distance in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Own,
    Creature(u64),
    Player(u64, f32),
    Other,
}

/// What the server must do with a prepared cast.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Instant vitals are in `commands()`; `mend` and `regen` go to encounters.
    Restore { health: u32, vigor: u32, mend: Option<Over>, regen: Option<Regen> },
    /// Vigor is in `commands()`; the target's vitals land on the next tick.
    Support { target: u64, health: u32, vigor: u32, mend: Option<Over>, regen: Option<Regen> },
    /// Queue one encounter strike; its vigor is spent when it lands.
    Strike { target: u64, bonus: u32, cost: u32, range: f32, burn: Option<Over> },
    /// Move the caster; vigor is in `commands()`.
    Travel(Travel),
}

pub struct Runtime {
    definitions: BTreeMap<u32, Definition>,
    skills: BTreeMap<(u64, u32), u64>,
    players: BTreeMap<u64, u64>,
    last_now: u64,
}
impl Runtime {
    pub fn load() -> Result<Self, AbilityError> {
        Self::parse(
            include_str!("../content/skill-effects.tsv"),
            include_str!("../content/skill-travel.tsv"),
        )
    }

    fn parse(text: &str, travel: &str) -> Result<Self, AbilityError> {
        let mut routes = BTreeMap::new();
        let mut travel_lines = travel.lines();
        if travel_lines.next() != Some("pyrealium-skill-travel-v1") {
            return Err(AbilityError::Catalog);
        }
        for line in travel_lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<_> = line.split('\t').collect();
            if f.len() != 3 {
                return Err(AbilityError::Catalog);
            }
            let skill = u32::from_str_radix(f[0], 16).map_err(|_| AbilityError::Catalog)?;
            let route = match f[2] {
                "lifestone" => Travel::Lifestone,
                "portal" => Travel::Portal,
                list => {
                    let mut pads = Vec::new();
                    for pad in list.split(';') {
                        let p: Vec<_> = pad.split(':').collect();
                        if p.len() != 5 || pads.len() >= 32 {
                            return Err(AbilityError::Catalog);
                        }
                        let number = |text: &str| text.parse::<f32>().map_err(|_| AbilityError::Catalog);
                        let placement = Placement {
                            cell: u32::from_str_radix(p[0], 16).map_err(|_| AbilityError::Catalog)?,
                            coordinates: [number(p[1])?, number(p[2])?, number(p[3])?],
                            heading: p[4].parse().map_err(|_| AbilityError::Catalog)?,
                        };
                        if !placement.valid() {
                            return Err(AbilityError::Catalog);
                        }
                        pads.push(placement);
                    }
                    Travel::Telepads(pads)
                }
            };
            routes.insert(skill, route);
        }

        let mut lines = text.lines();
        if lines.next() != Some("pyrealium-skill-effects-v2") {
            return Err(AbilityError::Catalog);
        }
        let mut definitions = BTreeMap::new();
        for line in lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<_> = line.split('\t').collect();
            if f.len() != 13 {
                return Err(AbilityError::Catalog);
            }
            let hex = |text: &str| u32::from_str_radix(text, 16).map_err(|_| AbilityError::Catalog);
            let number = |text: &str| text.parse::<f32>().map_err(|_| AbilityError::Catalog);
            let skill = hex(f[0])?;
            let kind = hex(f[2])?;
            let reset = number(f[3])?;
            let vigor: i64 = f[4].parse().map_err(|_| AbilityError::Catalog)?;
            let range = number(f[5])?;
            if !(0.0..=3600.0).contains(&reset) || !(-1.0..=1000.0).contains(&range) {
                return Err(AbilityError::Catalog);
            }
            let mut definition = Definition {
                kind,
                reset_ms: (reset * 1000.0) as u64,
                vigor: u32::try_from(vigor.unsigned_abs().min(100_000)).unwrap(),
                // -1 means the record gives no range; strikes then use melee range.
                range: range.max(0.0),
                targets: hex(f[6])?,
                right: hex(f[7])?,
                left: hex(f[8])?,
                story: hex(f[9])?,
                fx: hex(f[10])?,
                damage: Table::parse(f[11])?,
                health: Table::default(),
                vigor_effect: Table::default(),
                timed: Vec::new(),
                buff: None,
                ratings: Vec::new(),
                weapon: false,
                travel: routes.remove(&skill),
            };
            if f[12] != "-" {
                for effect in f[12].split(';') {
                    let p: Vec<_> = effect.split('/').collect();
                    if p.len() != 10 {
                        return Err(AbilityError::Catalog);
                    }
                    let side = if p[0] == "self" { Side::Own } else { Side::Target };
                    let kind = hex(p[2])?;
                    let period = number(p[5])?;
                    let duration = Table::parse(p[6])?;
                    let values = Table::parse(p[7])?;
                    match kind {
                        HEALTH if definition.health.is_empty() => definition.health = values,
                        VIGOR if definition.vigor_effect.is_empty() => definition.vigor_effect = values,
                        HEALTH_OVER_TIME | VIGOR_OVER_TIME
                            if !values.is_empty() && !duration.is_empty() && period > 0.0 =>
                        {
                            definition.timed.push(Timed {
                                side,
                                vigor: kind == VIGOR_OVER_TIME,
                                values,
                                period_ms: (period * 1000.0) as u64,
                                duration: duration.clone(),
                            });
                        }
                        WEAPON_ATTACK if definition.kind == BASIC_ATTACK_KIND => definition.weapon = true,
                        _ => {}
                    }
                    if p[8] != "-" && !duration.is_empty() || p[8].contains("40125") || p[8].contains("40262") || p[8].contains("80000") {
                        let mut health = Table::default();
                        let mut vigor = Table::default();
                        let mut rating = RatingSpec {
                            side,
                            duration: duration.clone(),
                            armor: Vec::new(),
                            attack: [Vec::new(), Vec::new(), Vec::new()],
                            defense: [Vec::new(), Vec::new(), Vec::new()],
                        };
                        if p[8] != "-" {
                            for stat in p[8].split('|') {
                                let (id, table) = stat.split_once('=').ok_or(AbilityError::Catalog)?;
                                let id = hex(id)?;
                                let parsed = Table::parse(table)?;
                                match id {
                                    HEALTH_REGEN if definition.buff.is_none() => health = parsed,
                                    VIGOR_REGEN if definition.buff.is_none() => vigor = parsed,
                                    NATURAL_ARMOR | ARMOR_LEVEL => rating.armor.push(parsed),
                                    _ => {
                                        if let Some(index) = ATTACK_RATING.iter().position(|stat| *stat == id) {
                                            rating.attack[index].push(parsed);
                                        } else if let Some(index) = DEFENSE_RATING.iter().position(|stat| *stat == id) {
                                            rating.defense[index].push(parsed);
                                        }
                                    }
                                }
                            }
                        }
                        if definition.buff.is_none() && (!health.is_empty() || !vigor.is_empty()) && !duration.is_empty() {
                            definition.buff = Some(Buff { side, health, vigor, duration });
                        }
                        if rating.useful() {
                            definition.ratings.push(rating);
                        }
                    }
                }
            }
            if definition.usable() && definitions.insert(skill, definition).is_some() {
                return Err(AbilityError::Catalog);
            }
        }
        if definitions.is_empty() {
            return Err(AbilityError::Catalog);
        }
        Ok(Self {
            definitions,
            skills: BTreeMap::new(),
            players: BTreeMap::new(),
            last_now: 0,
        })
    }

    pub fn supports(&self, skill: u32) -> bool {
        self.definitions.contains_key(&skill)
    }

    pub fn supported(&self) -> usize {
        self.definitions.len()
    }

    /// The skill's story behavior and FX, for presentation.
    pub fn presentation(&self, skill: u32) -> (u32, u32) {
        self.definitions
            .get(&skill)
            .map_or((0, 0), |d| (d.story, d.fx))
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

    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'a>(
        &'a mut self,
        player: u64,
        skill: u32,
        aim: Aim,
        hands: (Hand, Hand),
        book: &Book,
        state: &Progression,
        now: u64,
    ) -> Result<PreparedCast<'a>, AbilityError> {
        let definition = self.definitions.get(&skill).ok_or(AbilityError::Unsupported)?.clone();
        if player == 0 || player == u64::MAX {
            return Err(AbilityError::InvalidCharacter);
        }
        let rank = book.get(skill).ok_or(AbilityError::NotLearned)?.rank;
        if !state.valid() {
            return Err(AbilityError::InvalidProgression);
        }
        if !state.alive() {
            return Err(AbilityError::Dead);
        }
        if state.vigor < MIN_VIGOR && definition.vigor > 0 {
            return Err(AbilityError::InsufficientVigor);
        }
        if !holds(definition.right, hands.0) {
            return Err(AbilityError::Hand { right: true, required: definition.right });
        }
        if !holds(definition.left, hands.1) {
            return Err(AbilityError::Hand { right: false, required: definition.left });
        }
        let cost = definition.vigor.min(state.vigor);
        let over = |side: Side, vigor: bool, sign: f32| -> Option<Over> {
            definition
                .timed
                .iter()
                .find(|t| t.side == side && t.vigor == vigor && t.values.at(rank) * sign > 0.0)
                .map(|t| {
                    let amount = (t.values.at(rank) * sign).round() as u32;
                    Over {
                        health: if vigor { 0 } else { amount },
                        vigor: if vigor { amount } else { 0 },
                        period_ms: t.period_ms,
                        duration_ms: (t.duration.at(rank).max(0.0) * 1000.0) as u64,
                    }
                })
        };
        let mend_for = |side: Side| -> Option<Over> {
            let health = over(side, false, 1.0);
            let vigor = over(side, true, 1.0);
            match (health, vigor) {
                (Some(h), Some(v)) => Some(Over { vigor: v.vigor, ..h }),
                (h, v) => h.or(v),
            }
        };
        let regen_for = |side: Side| -> Option<Regen> {
            definition.buff.as_ref().filter(|b| b.side == side).map(|b| Regen {
                health: if b.health.is_empty() { 1.0 } else { b.health.at(rank) },
                vigor: if b.vigor.is_empty() { 1.0 } else { b.vigor.at(rank) },
                duration_ms: (b.duration.at(rank).max(0.0) * 1000.0) as u64,
            })
        };
        let restore = || {
            (
                definition.health.at(rank).max(0.0).round() as u32,
                definition.vigor_effect.at(rank).max(0.0).round() as u32,
            )
        };
        let caster_ratings = {
            let mut total = definition.ratings_for(Side::Own, rank);
            if matches!(aim, Aim::Own) {
                if let Some(targeted) = definition.ratings_for(Side::Target, rank) {
                    total = Some(match total {
                        Some((mut ratings, duration)) => {
                            ratings.add(targeted.0);
                            (ratings, duration.max(targeted.1))
                        }
                        None => targeted,
                    });
                }
            }
            total.filter(|(ratings, _)| !ratings.is_empty())
        };
        let other_ratings = match aim {
            Aim::Player(target, _) | Aim::Creature(target) => definition
                .ratings_for(Side::Target, rank)
                .filter(|(ratings, _)| !ratings.is_empty())
                .map(|(ratings, duration)| (target, ratings, duration)),
            _ => None,
        };
        let outcome = if let Some(travel) = definition.travel.clone() {
            Outcome::Travel(travel)
        } else {
            match aim {
                Aim::Creature(target) if definition.harmful() => {
                    if definition.targets & MONSTER == 0 {
                        return Err(AbilityError::InvalidTarget);
                    }
                    let instant = definition.health.at(rank).min(0.0).abs();
                    let bonus = (definition.damage.at(rank).abs() + instant).round() as u32;
                    Outcome::Strike {
                        target,
                        bonus,
                        cost,
                        range: definition.range,
                        burn: over(Side::Target, false, -1.0),
                    }
                }
                _ if definition.harmful() => return Err(AbilityError::TargetCreature),
                Aim::Own => {
                    if definition.targets & SELF == 0 {
                        return Err(AbilityError::InvalidTarget);
                    }
                    let (health, vigor) = restore();
                    Outcome::Restore {
                        health,
                        vigor,
                        mend: mend_for(Side::Target).or_else(|| mend_for(Side::Own)),
                        regen: regen_for(Side::Target).or_else(|| regen_for(Side::Own)),
                    }
                }
                Aim::Player(target, distance) => {
                    if definition.targets & PLAYER == 0 {
                        return Err(AbilityError::InvalidTarget);
                    }
                    if distance > definition.range.max(3.0) {
                        return Err(AbilityError::OutOfRange);
                    }
                    let (health, vigor) = restore();
                    Outcome::Support {
                        target,
                        health,
                        vigor,
                        mend: mend_for(Side::Target),
                        regen: regen_for(Side::Target),
                    }
                }
                Aim::Creature(_) | Aim::Other => return Err(AbilityError::InvalidTarget),
            }
        };
        if let Outcome::Support { health: 0, vigor: 0, mend: None, regen: None, .. } = outcome {
            if caster_ratings.is_none() && other_ratings.is_none() {
                return Err(AbilityError::Unsupported);
            }
        }
        self.expire(now)?;
        if let Some(&until) = self.skills.get(&(player, skill)) {
            return Err(AbilityError::Cooldown { remaining_ms: until - now });
        }
        if let Some(&until) = self.players.get(&player) {
            return Err(AbilityError::GlobalCooldown { remaining_ms: until - now });
        }
        if self.skills.len() >= MAX_SKILL_COOLDOWNS || self.players.len() >= MAX_PLAYER_COOLDOWNS {
            return Err(AbilityError::Capacity);
        }
        let until = now
            .checked_add(definition.reset_ms.max(GLOBAL_COOLDOWN_MS))
            .ok_or(AbilityError::Clock)?;
        let global_until = now.checked_add(GLOBAL_COOLDOWN_MS).ok_or(AbilityError::Clock)?;
        let mut commands = Vec::new();
        if !matches!(outcome, Outcome::Strike { .. }) && cost > 0 {
            commands.push(Command::SpendVigor(cost));
        }
        if let Outcome::Restore { health, vigor, .. } = outcome {
            if health > 0 {
                commands.push(Command::Heal(health));
            }
            if vigor > 0 {
                commands.push(Command::RecoverVigor(vigor));
            }
        }
        let next = commands.iter().try_fold(state.clone(), |value, command| {
            value.apply(command).map_err(|_| AbilityError::InvalidProgression)
        })?;
        if let Outcome::Restore { mend: None, regen: None, .. } = outcome {
            if next.health == state.health && next.vigor + cost <= state.vigor && caster_ratings.is_none() {
                return Err(AbilityError::NothingToRestore);
            }
        }
        Ok(PreparedCast {
            runtime: self,
            player,
            skill,
            // Basic weapon attacks keep the validated training swing presentation.
            story: if definition.weapon { 0 } else { definition.story },
            fx: if definition.weapon { 0 } else { definition.fx },
            until,
            global_until,
            outcome,
            commands,
            previous: state.clone(),
            next,
            caster_ratings,
            other_ratings,
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
    story: u32,
    fx: u32,
    until: u64,
    global_until: u64,
    outcome: Outcome,
    commands: Vec<Command>,
    previous: Progression,
    next: Progression,
    caster_ratings: Option<(crate::combat::Ratings, u64)>,
    other_ratings: Option<(u64, crate::combat::Ratings, u64)>,
}
impl PreparedCast<'_> {
    pub fn player(&self) -> u64 {
        self.player
    }
    pub fn skill(&self) -> u32 {
        self.skill
    }
    /// Story behavior and FX from the skill record (0 = none).
    pub fn presentation(&self) -> (u32, u32) {
        (self.story, self.fx)
    }
    pub fn outcome(&self) -> &Outcome {
        &self.outcome
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
    pub fn caster_ratings(&self) -> Option<(crate::combat::Ratings, u64)> {
        self.caster_ratings
    }
    pub fn other_ratings(&self) -> Option<(u64, crate::combat::Ratings, u64)> {
        self.other_ratings
    }
    pub fn healed(&self) -> u32 {
        self.next.health - self.previous.health
    }
    pub fn ready_at(&self) -> u64 {
        self.until
    }
    /// Call only after the atomic progression batch has been durably saved.
    pub fn commit(self) {
        self.runtime.skills.insert((self.player, self.skill), self.until);
        self.runtime.players.insert(self.player, self.global_until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_skill_records_drive_heal_and_strike_values() {
        let runtime = Runtime::load().unwrap();
        let ease = &runtime.definitions[&0x55];
        assert_eq!((ease.reset_ms, ease.vigor, ease.range, ease.targets), (7000, 100, 40.0, 0x101));
        assert_eq!((ease.health.at(1), ease.health.at(50)), (25.0, 250.0));
        let bolt = &runtime.definitions[&0x59];
        assert_eq!((bolt.reset_ms, bolt.vigor, bolt.range, bolt.right, bolt.left), (1000, 32, 60.0, 0x40000, 1));
        assert_eq!((bolt.damage.at(1), bolt.damage.at(50)), (3.0, 75.0));
        let burn = &bolt.timed[0];
        assert_eq!((burn.values.at(1), burn.values.at(50), burn.period_ms), (-1.0, -4.0, 1000));
        assert_eq!(burn.duration.at(1), 30.0);
        assert!(bolt.harmful() && !ease.harmful());
        let defense = runtime.definitions[&0x6b].ratings_for(Side::Own, 1).unwrap();
        assert_eq!(defense.0.attack(crate::combat::School::Melee), 7);
        assert_eq!(defense.1, 180_000);
        let aura = runtime.definitions[&0xa1].ratings_for(Side::Target, 50).unwrap();
        assert_eq!(aura.0.defense(crate::combat::School::Magic), 100);
        let calm = &runtime.definitions[&0x67];
        assert_eq!(calm.buff.as_ref().map(|b| (b.vigor.at(1), b.vigor.at(50))), Some((1.1, 2.0)));
        assert!(runtime.definitions[&0xb0].weapon);
        assert_eq!(runtime.definitions[&0x343].travel, Some(Travel::Lifestone));
        assert_eq!(runtime.definitions[&0x366].travel, Some(Travel::Portal));
        assert!(matches!(runtime.definitions[&0x39d].travel, Some(Travel::Telepads(ref pads)) if pads[0].cell == 0x6ea70025));
    }

    #[test]
    fn hands_follow_the_implement_masks() {
        assert!(holds(0x40000, Hand::Implement(0x40000)));
        assert!(!holds(0x40000, Hand::Implement(0x2)));
        assert!(!holds(0x40000, Hand::Empty));
        assert!(holds(0x1, Hand::Empty));
        assert!(holds(0x80073, Hand::Empty));
        assert!(holds(0xa0062, Hand::Implement(0x2)));
        assert!(holds(0xffffffff, Hand::Empty));
        assert!(!holds(0xfffffffe, Hand::Empty));
        assert!(holds(0x40000, Hand::Unknown));
    }

    #[test]
    fn table_interpolates_between_level_points_and_clamps_outside() {
        let table = Table::parse("1:3,50:75,150:500").unwrap();
        assert_eq!(table.at(0), 3.0);
        assert!((table.at(25) - 38.265).abs() < 0.01);
        assert_eq!(table.at(150), 500.0);
        assert_eq!(table.at(400), 500.0);
    }
}
