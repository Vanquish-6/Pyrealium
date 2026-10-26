//! Combat resolution shared by player and creature attacks.
//!
//! Ported from the 2012 world server (`3 emu2012`): creature natural damage
//! rolls (`Monster.DecisionTree`), armor mitigation with its 25% floor
//! (`Living.ModifyHealth`) and player attack/defense ratings
//! (`Player.CalculateAttackDefenseRatings`). Creature ratings are the
//! portal.dat starting-skill levels for magic/melee/missile attack and defense.
//!
//! The hit-chance curve is authored: neither the legacy server nor the
//! recovered 7148 client contains the original formula. Equal ratings hit 75%
//! of the time, and the chance stays between 5% and 95%.
use crate::skills::Book;

/// Per-school mastery skills (SkillList): master, grandmaster, paragon.
const MELEE: [u32; 3] = [0xa, 0x9, 0xb];
const MISSILE: [u32; 3] = [0xf, 0xe, 0x10];
const MAGIC: [u32; 3] = [0x4, 0x3, 0x5];

/// xorshift64* seeded from the OS; combat rolls need no cryptographic strength.
pub struct Dice(u64);
impl Dice {
    pub fn seeded() -> Self {
        let mut seed = [0u8; 8];
        let _ = getrandom::getrandom(&mut seed);
        Self(u64::from_le_bytes(seed) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in `low..=high`; returns `low` when the range is empty.
    pub fn between(&mut self, low: u32, high: u32) -> u32 {
        if high <= low {
            return low;
        }
        low + (self.next() % u64::from(high - low + 1)) as u32
    }
    pub fn roll(&mut self, chance: f32) -> bool {
        self.unit() < f64::from(chance)
    }
}

/// `Random.Next(damage - damage * variance, damage)`: the upper bound is exclusive.
pub fn natural_damage(damage: u32, variance: f32, dice: &mut Dice) -> u32 {
    let spread = (damage as f32 * variance.clamp(0.0, 1.0)) as u32;
    let low = damage - spread.min(damage);
    dice.between(low, damage.saturating_sub(1).max(low)).max(1)
}

/// Armor and shield subtract from the hit. A result under 25% of the raw hit
/// becomes a random value from 1 up to (excluding) that threshold.
pub fn mitigate(raw: u32, armor: u32, dice: &mut Dice) -> u32 {
    let threshold = (f64::from(raw) * 0.25).round_ties_even() as u32;
    let reduced = raw.saturating_sub(armor);
    if reduced < threshold {
        dice.between(1, threshold.saturating_sub(1).max(1))
    } else {
        reduced.max(1)
    }
}

fn rank(book: &Book, skill: u32) -> f32 {
    book.get(skill).map_or(0.0, |trained| f32::from(trained.rank.max(1)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum School {
    Melee,
    Missile,
    Magic,
}
impl School {
    fn skills(self) -> [u32; 3] {
        match self {
            Self::Melee => MELEE,
            Self::Missile => MISSILE,
            Self::Magic => MAGIC,
        }
    }
}

/// Level * 8 plus mastery: attack 2.4/4.4/6.4 per rank, defense 2/4/6.
pub fn player_attack(level: u16, book: &Book, school: School) -> u32 {
    let [master, grand, paragon] = school.skills();
    (f32::from(level) * 8.0
        + rank(book, master) * 2.4
        + rank(book, grand) * 4.4
        + rank(book, paragon) * 6.4) as u32
}
pub fn player_defense(level: u16, book: &Book, school: School) -> u32 {
    let [master, grand, paragon] = school.skills();
    (f32::from(level) * 8.0
        + rank(book, master) * 2.0
        + rank(book, grand) * 4.0
        + rank(book, paragon) * 6.0) as u32
}

pub fn hit_chance(attack: u32, defense: u32) -> f32 {
    let (a, d) = (attack as f32, defense as f32);
    if a + d <= 0.0 {
        return 0.75;
    }
    (0.75 + 0.5 * (a - d) / (a + d)).clamp(0.05, 0.95)
}

/// Shortest native heading (255 units per clockwise revolution from +Y) that
/// faces from `from` toward `to`. See `Placement::orientation`.
pub fn heading(from: [f32; 3], to: [f32; 3]) -> Option<u8> {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    if dx * dx + dy * dy < 1e-6 {
        return None;
    }
    let turns = dx.atan2(dy) / std::f32::consts::TAU;
    Some(((turns.rem_euclid(1.0) * 255.0).round() as u32 % 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mitigation_matches_legacy_floor() {
        let mut dice = Dice(7);
        assert_eq!(mitigate(100, 10, &mut dice), 90);
        for _ in 0..64 {
            let low = mitigate(100, 90, &mut dice);
            assert!((1..25).contains(&low));
        }
        assert_eq!(mitigate(1, 50, &mut dice), 1);
    }
    #[test]
    fn natural_damage_stays_in_legacy_band() {
        let mut dice = Dice(11);
        for _ in 0..256 {
            assert!((82..117).contains(&natural_damage(117, 0.3, &mut dice)));
        }
        assert_eq!(natural_damage(1, 0.7, &mut dice), 1);
    }
    #[test]
    fn hit_chance_is_bounded_and_even_at_parity() {
        assert_eq!(hit_chance(100, 100), 0.75);
        assert!(hit_chance(8, 2000) >= 0.05);
        assert!(hit_chance(2000, 8) <= 0.95);
    }
    #[test]
    fn heading_matches_native_axes() {
        assert_eq!(heading([0.0; 3], [0.0, 1.0, 0.0]), Some(0));
        assert_eq!(heading([0.0; 3], [1.0, 0.0, 0.0]), Some(64));
        assert_eq!(heading([0.0; 3], [0.0, -1.0, 0.0]), Some(128));
        assert_eq!(heading([0.0; 3], [-1.0, 0.0, 0.0]), Some(191));
    }
}
