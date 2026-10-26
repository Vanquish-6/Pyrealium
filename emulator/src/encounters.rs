//! Server-run encounters: creature AI, combat resolution, corpses and loot.
//!
//! Creature stats come from portal.dat (`content/creature-combat.tsv`) unless an
//! authored development row overrides them. Hit, damage and armor rolls are in
//! `combat.rs`. Creatures acquire players inside their DAT perception radius,
//! chase in a straight line inside their own cell at the observed retail cadence
//! (`tools/survey_server_motion.py`), and return home past their homesick
//! radius, following the 2012 `Monster.DecisionTree`. There is no pathfinding,
//! collision or line-of-sight test. Loot tables are authored.
use crate::{
    combat::{self, Dice, School},
    content::Content,
    merchants::Stock,
    movement::Placement,
    progression::{Command, Progression},
    scene::Scene,
    store::Store,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::Path,
};
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}
#[derive(Clone, Copy)]
struct Rule {
    health: u32,
    damage: u32,
    armor: u32,
    interval: u64,
    range: f32,
    respawn: u64,
    xp: u64,
    gold: u32,
    variance: f32,
    attack: u32,
    defense: u32,
    /// 0 = only fights back when attacked.
    perception: f32,
    homesick: f32,
}
/// Authored profile for rows without DAT combat data: level 1 ratings,
/// retaliation only, a short leash.
const AUTHORED_VARIANCE: f32 = 0.2;
const AUTHORED_RATING: u32 = 8;
const AUTHORED_HOMESICK: f32 = 20.0;
/// `Monster` constructor default when the record has no AI_HomesickRadius.
const DEFAULT_HOMESICK: f32 = 60.0;
/// Authored spread on player weapon hits; no player weapon damage table is recovered.
const PLAYER_VARIANCE: f32 = 0.2;
const MOVE_MS: u64 = 300;
/// Retail chasers in `large.combat.elog` covered about 6.6 m/s; slightly slower here.
const CHASE_SPEED: f32 = 5.0;
const RUN_MOTION: u32 = 0x7ffe7f;
const STAND_MOTION: u32 = 0x7f7f7f;
const CORPSE_MS: u64 = 120_000;
const MAX_CORPSES: usize = 256;
pub const CORPSE_NAMESPACE: u64 = 0x8140_f000_0000_0000;
struct Actor {
    entity: u32,
    rule: Rule,
    health: u32,
    target: Option<u64>,
    next_attack: u64,
    respawn_at: Option<u64>,
    despawn_at: Option<u64>,
    home: Placement,
    next_move: u64,
    moving: bool,
    returning: bool,
    /// Killer and loot waiting for the corpse to appear.
    remains: Option<(u64, Loot)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loot {
    pub gold: u32,
    pub item: Option<Stock>,
}
pub struct Corpse {
    pub killer: u64,
    pub loot: Loot,
    expires: u64,
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b) * (a - b)).sum::<f32>().sqrt()
}
#[derive(Clone, Copy)]
struct Attack {
    target: u64,
    next: u64,
    repeat: bool,
    strike: Option<Strike>,
}
/// One skill hit: weapon damage plus `bonus`, spending up to `cost` vigor.
/// `behavior` and `fx` are the skill record's story behavior and FX (0 = none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub bonus: u32,
    pub cost: u32,
    pub range: f32,
    pub behavior: u32,
    pub fx: u32,
    pub burn: Option<Over>,
}
/// A vital change applied every `period_ms` for `duration_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Over {
    pub health: u32,
    pub vigor: u32,
    pub period_ms: u64,
    pub duration_ms: u64,
}
/// Regeneration multipliers from a timed buff (1.0 = unchanged).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Regen {
    pub health: f32,
    pub vigor: f32,
    pub duration_ms: u64,
}
struct Burn {
    source: u64,
    target: u64,
    per_tick: u32,
    period: u64,
    next: u64,
    until: u64,
}
struct Mend {
    source: u64,
    player: u64,
    health: u32,
    vigor: u32,
    period: u64,
    next: u64,
    until: u64,
    /// FX shown on the player when the first restoration lands (0 = none).
    fx: u32,
}
const MAX_TIMED: usize = 512;
pub enum Event {
    IdolVisual(u64, crate::combat_visual::IdolAction),
    /// A landed attack. `behavior` is a skill's story behavior; None plays the swing.
    Swing {
        player: u64,
        behavior: Option<u32>,
    },
    /// Skill FX on a creature or player.
    Fx {
        target: u64,
        fx: u32,
    },
    /// Signed health on the victim. Negative is damage. The vital update is separate.
    Hit {
        victim: u64,
        attacker: u64,
        health_change: i32,
    },
    /// Same-cell creature movement; `motion` is the native low 24-bit motion vector.
    Move {
        actor: u64,
        placement: Placement,
        motion: u32,
    },
    /// Create corpse `id` where `creature` stands; only `killer` may loot it.
    Corpse {
        id: u64,
        creature: u64,
        killer: u64,
    },
    CorpseExpired(u64),
    Quest {
        player: u64,
        book: crate::quests::Book,
    },
    Health {
        actor: u64,
        health: u32,
    },
    Despawn(u64),
    Respawn {
        actor: u64,
        health: u32,
    },
    Owner {
        player: u64,
        previous: Progression,
        state: Progression,
    },
    Note {
        player: u64,
        text: String,
    },
    Stop(u64),
}
pub struct Encounters {
    actors: BTreeMap<u64, Actor>,
    attacks: BTreeMap<u64, Attack>,
    cooldowns: BTreeMap<u64, u64>,
    damage: u32,
    cost: u32,
    interval: u64,
    range: f32,
    next_tick: u64,
    regen_health: u32,
    regen_vigor: u32,
    regen_interval: u64,
    next_regen: u64,
    training_loot: bool,
    burns: Vec<Burn>,
    mends: Vec<Mend>,
    regen: BTreeMap<u64, (Regen, u64)>,
    dice: Dice,
    corpses: BTreeMap<u64, Corpse>,
    next_corpse: u64,
}
fn in_range(a: Option<Placement>, b: Option<Placement>, range: f32) -> bool {
    a.zip(b).is_some_and(|(a, b)| {
        a.cell == b.cell
            && a.coordinates
                .iter()
                .zip(b.coordinates)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                <= range * range
    })
}
impl Encounters {
    pub fn load(path: &Path, content: &Content) -> io::Result<Self> {
        if path == Path::new("training") {
            return Self::parse(
                include_str!("../content/development-encounters.tsv"),
                content,
            );
        }
        let mut bytes = Vec::new();
        File::open(path)?.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(invalid("encounter profile exceeds 64 KiB"));
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| invalid("encounter profile encoding"))?;
        Self::parse(text, content)
    }
    fn parse(text: &str, content: &Content) -> io::Result<Self> {
        if text.len() > 65536 {
            return Err(invalid("encounter profile exceeds 64 KiB"));
        }
        let mut lines = text.lines();
        if lines.next() != Some("pyrealium-encounters-v1") {
            return Err(invalid("encounter profile header"));
        }
        let mut player = None;
        let mut regen = None;
        let mut rules = BTreeMap::new();
        for line in lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.len() > 512 {
                return Err(invalid("encounter row length"));
            }
            let f: Vec<_> = line.split('\t').collect();
            let number = |i: usize| {
                f[i].parse::<u64>()
                    .map_err(|_| invalid("invalid encounter integer"))
            };
            let small = |i: usize| {
                u32::try_from(number(i)?).map_err(|_| invalid("encounter integer overflow"))
            };
            let range = |i: usize| -> io::Result<f32> {
                let value = f[i]
                    .parse::<f32>()
                    .map_err(|_| invalid("encounter range"))?;
                if !value.is_finite() || !(0.5..=30.0).contains(&value) {
                    return Err(invalid("encounter range bounds"));
                }
                Ok(value)
            };
            match f.as_slice() {
                ["player", _, _, _, _] if player.is_none() => {
                    player = Some((small(1)?, small(2)?, number(3)?, range(4)?));
                }
                ["regen", _, _, _] if regen.is_none() => {
                    regen = Some((small(1)?, small(2)?, number(3)?));
                }
                ["npc", _, _, _, _, _, _, _, _] => {
                    let entity = u32::from_str_radix(f[1].trim_start_matches("0x"), 16)
                        .map_err(|_| invalid("encounter entity"))?;
                    let rule = Rule {
                        health: small(2)?,
                        damage: small(3)?,
                        armor: 0,
                        interval: number(4)?,
                        range: range(5)?,
                        respawn: number(6)?,
                        xp: number(7)?,
                        gold: small(8)?,
                        variance: AUTHORED_VARIANCE,
                        attack: AUTHORED_RATING,
                        defense: AUTHORED_RATING,
                        perception: 0.0,
                        homesick: AUTHORED_HOMESICK,
                    };
                    if entity >> 24 != 0x47
                        || rule.health == 0
                        || rule.health > 1_000_000
                        || rule.damage == 0
                        || rule.damage > 1_000_000
                        || !(250..=60000).contains(&rule.interval)
                        || !(1000..=86400000).contains(&rule.respawn)
                        || rule.xp > i64::MAX as u64
                        || rule.gold > i32::MAX as u32
                        || rules.len() >= 256
                        || rules.insert(entity, rule).is_some()
                    {
                        return Err(invalid("invalid or duplicate encounter rule"));
                    }
                }
                _ => return Err(invalid("unrecognized encounter row")),
            }
        }
        let (damage, cost, interval, range) =
            player.ok_or_else(|| invalid("missing player attack profile"))?;
        let (regen_health, regen_vigor, regen_interval) = regen.unwrap_or((0, 0, 1000));
        if damage == 0
            || damage > 1_000_000
            || cost == 0
            || cost > 1_000_000
            || !(250..=60000).contains(&interval)
            || regen_health > 1_000_000
            || regen_vigor > 1_000_000
            || !(1000..=60000).contains(&regen_interval)
        {
            return Err(invalid("player or regeneration profile bounds"));
        }
        // An authored row wins: the retail Idol Spawn is level 24 and one-hits a
        // level 1 character. Interval 0 means the record had no natural weapon
        // time. Creatures with no combat record keep the starter profile.
        for line in include_str!("../content/creature-combat.tsv").lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("pyrealium-") {
                continue;
            }
            let f: Vec<_> = line.split('\t').collect();
            if f.len() != 13 {
                return Err(invalid("creature combat field count"));
            }
            let real = |i: usize, max: f32| -> io::Result<f32> {
                let value: f32 = f[i].parse().map_err(|_| invalid("creature combat decimal"))?;
                if !value.is_finite() || !(0.0..=max).contains(&value) {
                    return Err(invalid("creature combat decimal bounds"));
                }
                Ok(value)
            };
            let entity = u32::from_str_radix(f[0], 16).map_err(|_| invalid("creature combat entity"))?;
            let number = |i: usize| {
                f[i].parse::<u64>()
                    .map_err(|_| invalid("creature combat integer"))
            };
            let small = |i: usize| {
                u32::try_from(number(i)?).map_err(|_| invalid("creature combat overflow"))
            };
            let health = small(1)?;
            let damage = small(2)?;
            let mut interval = number(3)?;
            if interval == 0 {
                interval = 1500;
            }
            let armor = small(4)?;
            let xp = number(5)?;
            let level = small(6)?;
            let variance = real(7, 1.0)?;
            let attack = small(8)?;
            let defense = small(9)?;
            let perception = real(10, 500.0)?;
            let homesick = real(11, 500.0)?;
            if entity >> 24 != 0x47
                || level > 1000
                || attack > 1_000_000
                || defense > 1_000_000
                || health == 0
                || health > 5_000_000
                || damage == 0
                || damage > 5_000_000
                || armor > 1_000_000
                || !(250..=60000).contains(&interval)
                || xp > i64::MAX as u64
            {
                return Err(invalid("creature combat bounds"));
            }
            let rule = Rule {
                health,
                damage,
                armor,
                interval,
                range: 3.0,
                respawn: 30_000,
                xp,
                // Authored: no creature gold table is recovered.
                gold: 2 + level / 2,
                variance,
                // A record without combat skills keeps its level-equivalent rating.
                attack: if attack == 0 { level.max(1) * 8 } else { attack },
                defense: if defense == 0 { level.max(1) * 8 } else { defense },
                perception,
                homesick: if homesick > 0.0 { homesick } else { DEFAULT_HOMESICK },
            };
            if rules.contains_key(&entity) {
                continue;
            }
            if rules.len() >= 4096 || rules.insert(entity, rule).is_some() {
                return Err(invalid("creature combat rule limit"));
            }
        }
        let starter = Rule {
            health: 60,
            damage: 6,
            armor: 0,
            interval: 1500,
            range: 3.0,
            respawn: 30_000,
            xp: 40,
            gold: 2,
            variance: AUTHORED_VARIANCE,
            attack: AUTHORED_RATING,
            defense: AUTHORED_RATING,
            perception: 0.0,
            homesick: AUTHORED_HOMESICK,
        };
        for line in include_str!("../content/creature-entities.txt").lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("pyrealium-") {
                continue;
            }
            let entity = u32::from_str_radix(line, 16).map_err(|_| invalid("creature entity"))?;
            rules.entry(entity).or_insert(starter);
        }
        // The mentor's entity is a unique quest monster record (AI_UniqueMonster,
        // NPC_IsGroupMonster); placed as a service NPC it must never fight.
        let service = [crate::training_dialogue::MENTOR_ENTITY, crate::merchants::MERCHANT_ENTITY];
        let actors = content
            .objects()
            .filter(|object| !service.contains(&object.entity))
            .filter_map(|object| {
                rules.get(&object.entity).map(|&rule| {
                    (
                        object.id,
                        Actor {
                            entity: object.entity,
                            rule,
                            health: rule.health,
                            target: None,
                            next_attack: 0,
                            respawn_at: None,
                            despawn_at: None,
                            home: object.placement,
                            next_move: 0,
                            moving: false,
                            returning: false,
                            remains: None,
                        },
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        if actors.is_empty() || actors.len() > 4096 {
            return Err(invalid("encounters require 1..4096 matching placed actors"));
        }
        Ok(Self {
            actors,
            attacks: BTreeMap::new(),
            cooldowns: BTreeMap::new(),
            damage,
            cost,
            interval,
            range,
            next_tick: 0,
            regen_health,
            regen_vigor,
            regen_interval,
            next_regen: 0,
            training_loot: false,
            burns: Vec::new(),
            mends: Vec::new(),
            regen: BTreeMap::new(),
            dice: Dice::seeded(),
            corpses: BTreeMap::new(),
            next_corpse: CORPSE_NAMESPACE | 1,
        })
    }
    pub fn corpse(&self, id: u64) -> Option<&Corpse> {
        self.corpses.get(&id)
    }
    /// Record what the killer collected; an emptied corpse is removed.
    /// Returns true when the corpse should be destroyed.
    pub fn looted(&mut self, id: u64, took_gold: bool, took_item: bool) -> bool {
        let Some(corpse) = self.corpses.get_mut(&id) else {
            return false;
        };
        if took_gold {
            corpse.loot.gold = 0;
        }
        if took_item {
            corpse.loot.item = None;
        }
        if corpse.loot.gold == 0 && corpse.loot.item.is_none() {
            self.corpses.remove(&id);
            return true;
        }
        false
    }
    /// The corpse could not be created; hand back its loot.
    pub fn discard_corpse(&mut self, id: u64) -> Option<Corpse> {
        self.corpses.remove(&id)
    }
    /// Authored drop table; the DAT loot profiles reference an unrecovered
    /// treasure system. Items are the validated training stock.
    fn roll_loot(&mut self, rule: &Rule) -> Loot {
        let roll = self.dice.unit();
        let item = if roll < 0.30 {
            Some(Stock::Potion)
        } else if roll < 0.45 {
            Some(Stock::Mucor)
        } else if roll < 0.55 {
            Some(Stock::Tallow)
        } else {
            None
        };
        Loot { gold: rule.gold, item }
    }
    /// Despawn a defeated creature, leaving its corpse first.
    fn bury(&mut self, id: u64, now: u64, events: &mut Vec<Event>) {
        if let Some((killer, loot)) = self.actors.get_mut(&id).and_then(|actor| actor.remains.take()) {
            if self.corpses.len() >= MAX_CORPSES {
                if let Some((&oldest, _)) = self.corpses.iter().min_by_key(|(_, corpse)| corpse.expires) {
                    self.corpses.remove(&oldest);
                    events.push(Event::CorpseExpired(oldest));
                }
            }
            let corpse = self.next_corpse;
            self.next_corpse = CORPSE_NAMESPACE | ((self.next_corpse + 1) & 0x0fff_ffff_ffff);
            self.corpses.insert(corpse, Corpse { killer, loot, expires: now.saturating_add(CORPSE_MS) });
            events.push(Event::Corpse { id: corpse, creature: id, killer });
        }
        events.push(Event::Despawn(id));
    }
    pub fn initial_health(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.actors
            .iter()
            .map(|(&id, actor)| (id, actor.rule.health))
    }
    pub fn stop(&mut self, player: u64) {
        self.attacks.remove(&player);
    }
    pub fn attacking_target(&self, player: u64) -> Option<u64> {
        self.attacks.get(&player).map(|attack| attack.target)
    }
    pub fn is_creature(&self, id: u64) -> bool {
        self.actors.contains_key(&id)
    }
    pub fn includes_entity(&self, entity: u32) -> bool {
        self.actors.values().any(|actor| actor.entity == entity)
    }
    /// Call only after the server validates the authored potion's DAT profile.
    pub fn enable_training_loot(&mut self) {
        self.training_loot = true;
    }
    pub fn start(
        &mut self,
        player: u64,
        target: u64,
        now: u64,
        scene: &Scene,
        store: &Store,
    ) -> io::Result<()> {
        let actor = self
            .actors
            .get(&target)
            .filter(|actor| actor.health > 0)
            .ok_or_else(|| invalid("target is not an attackable creature"))?;
        if !store
            .characters
            .get(&player)
            .is_some_and(|c| !c.deleted && c.progression.alive())
        {
            return Err(invalid("dead or unavailable character cannot attack"));
        }
        if !scene.can_observe(player, target)
            || !in_range(
                scene.player_placement(player),
                scene.content_placement(target),
                self.range,
            )
        {
            return Err(invalid("target is out of attack range"));
        }
        if actor.respawn_at.is_some()
            || self.attacks.len() >= 64 && !self.attacks.contains_key(&player)
        {
            return Err(invalid("encounter attack capacity"));
        }
        self.cooldowns.retain(|_, until| *until > now);
        if self.cooldowns.len() >= 128 && !self.cooldowns.contains_key(&player) {
            return Err(invalid("attack cooldown capacity"));
        }
        let next = self.cooldowns.get(&player).copied().unwrap_or(now);
        self.attacks.insert(
            player,
            Attack {
                target,
                next,
                repeat: true,
                strike: None,
            },
        );
        Ok(())
    }
    /// Queue one skill hit that lands on the next tick. A running auto-attack on
    /// the same target resumes afterwards; skill cooldowns are the caller's.
    pub fn strike(
        &mut self,
        player: u64,
        target: u64,
        now: u64,
        scene: &Scene,
        store: &Store,
        strike: Strike,
    ) -> io::Result<()> {
        if !strike.range.is_finite() || strike.range < 0.0 {
            return Err(invalid("invalid skill range"));
        }
        if !self
            .actors
            .get(&target)
            .is_some_and(|actor| actor.health > 0 && actor.respawn_at.is_none())
        {
            return Err(invalid("target is not an attackable creature"));
        }
        if !store
            .characters
            .get(&player)
            .is_some_and(|c| !c.deleted && c.progression.alive())
        {
            return Err(invalid("dead or unavailable character cannot attack"));
        }
        if !scene.can_observe(player, target)
            || !in_range(
                scene.player_placement(player),
                scene.content_placement(target),
                strike.range.max(self.range),
            )
        {
            return Err(invalid("target is out of skill range"));
        }
        let resume = self
            .attacks
            .get(&player)
            .is_some_and(|attack| attack.repeat && attack.target == target);
        if self.attacks.len() >= 64 && !self.attacks.contains_key(&player) {
            return Err(invalid("encounter attack capacity"));
        }
        self.attacks.insert(
            player,
            Attack {
                target,
                next: now,
                repeat: resume,
                strike: Some(strike),
            },
        );
        Ok(())
    }
    /// One queued strike shares the normal cooldown, payment and target checks.
    pub fn once(
        &mut self,
        player: u64,
        target: u64,
        now: u64,
        scene: &Scene,
        store: &Store,
    ) -> io::Result<()> {
        self.start(player, target, now, scene, store)?;
        self.attacks.get_mut(&player).unwrap().repeat = false;
        Ok(())
    }
    fn stopped(&mut self, player: u64, events: &mut Vec<Event>) {
        self.attacks.remove(&player);
        events.push(Event::Stop(player));
    }
    pub fn tick(
        &mut self,
        now: u64,
        scene: &Scene,
        store: &mut Store,
        active: impl Iterator<Item = u64>,
    ) -> io::Result<Vec<Event>> {
        if now < self.next_tick {
            return Ok(Vec::new());
        }
        self.next_tick = now.saturating_add(100);
        let mut events = Vec::new();
        let due = self
            .attacks
            .iter()
            .filter(|(_, attack)| attack.next <= now)
            .map(|(&id, &attack)| (id, attack))
            .collect::<Vec<_>>();
        for (player, attack) in due {
            if !self.attacks.contains_key(&player) {
                continue;
            }
            let available = self
                .actors
                .get(&attack.target)
                .is_some_and(|actor| actor.health > 0);
            let alive = store
                .characters
                .get(&player)
                .is_some_and(|c| !c.deleted && c.progression.alive());
            let range = attack.strike.map_or(self.range, |strike| strike.range.max(self.range));
            if !available
                || !alive
                || !in_range(
                    scene.player_placement(player),
                    scene.content_placement(attack.target),
                    range,
                )
            {
                if alive {
                    events.push(Event::Note {
                        player,
                        text: if available {
                            "Attack stopped: move closer to your target."
                        } else {
                            "Attack stopped: your target is no longer available."
                        }.into(),
                    });
                }
                self.stopped(player, &mut events);
                continue;
            }
            let (defense, armor, health) = {
                let actor = &self.actors[&attack.target];
                (actor.rule.defense, actor.rule.armor, actor.health)
            };
            let character = &store.characters[&player];
            let level = character.progression.level().number;
            let rating = if attack.strike.is_some() {
                [School::Melee, School::Missile, School::Magic]
                    .into_iter()
                    .map(|school| combat::player_attack(level, &character.skills, school))
                    .max()
                    .unwrap_or(0)
            } else {
                combat::player_attack(level, &character.skills, School::Melee)
            };
            let hit = self.dice.roll(combat::hit_chance(rating, defense));
            let damage = attack
                .strike
                .map_or(self.damage, |strike| self.damage.saturating_add(strike.bonus));
            let raw = combat::natural_damage(damage, PLAYER_VARIANCE, &mut self.dice);
            let dealt = combat::mitigate(raw, armor, &mut self.dice).min(health);
            // A skill drains up to its cost; it only needs the retail minimum of 1.
            let cost = match attack.strike {
                Some(strike) => strike
                    .cost
                    .min(store.characters[&player].progression.vigor),
                None => self.cost,
            };
            if attack.strike.is_some() && store.characters[&player].progression.vigor == 0 {
                events.push(Event::Note {
                    player,
                    text: "You do not have enough vigor for this skill.".into(),
                });
                self.stopped(player, &mut events);
                continue;
            }
            if !hit {
                if attack.strike.is_some() && cost > 0 {
                    let account = store.characters[&player].account.clone();
                    let previous = store.characters[&player].progression.clone();
                    store.progression_batch(&account, player, &[Command::SpendVigor(cost)])?;
                    events.push(Event::Owner {
                        player,
                        previous,
                        state: store.characters[&player].progression.clone(),
                    });
                }
                self.provoke(attack.target, player, now, &mut events);
                events.push(Event::Note { player, text: "You miss.".into() });
            } else if !self.land(player, attack.target, dealt, cost, true, now, store, &mut events)? {
                self.stopped(player, &mut events);
                continue;
            }
            let until = now.saturating_add(self.interval);
            self.cooldowns.insert(player, until);
            if let Some(queued) = self.attacks.get_mut(&player) {
                queued.next = until;
            }
            events.push(Event::Swing {
                player,
                behavior: attack.strike.map(|strike| strike.behavior).filter(|&b| b != 0),
            });
            if let Some(strike) = attack.strike.filter(|_| hit) {
                if strike.fx != 0 {
                    events.push(Event::Fx {
                        target: attack.target,
                        fx: strike.fx,
                    });
                }
                if let Some(burn) = strike.burn.filter(|_| self.actors[&attack.target].health > 0) {
                    self.burns
                        .retain(|b| !(b.source == player && b.target == attack.target));
                    if self.burns.len() < MAX_TIMED && burn.health > 0 {
                        let period = burn.period_ms.max(250);
                        self.burns.push(Burn {
                            source: player,
                            target: attack.target,
                            per_tick: burn.health,
                            period,
                            next: now.saturating_add(period),
                            until: now.saturating_add(burn.duration_ms),
                        });
                    }
                }
            }
            if !attack.repeat {
                self.attacks.remove(&player);
            } else if let Some(resumed) = self.attacks.get_mut(&player) {
                resumed.strike = None;
            }
        }
        let active: Vec<u64> = active.collect();
        self.tick_burns(now, store, &mut events)?;
        self.tick_mends(now, scene, store, &mut events)?;
        self.tick_creatures(now, scene, store, &active, &mut events)?;
        self.tick_regen(now, scene, store, active.into_iter(), &mut events)?;
        let expired: Vec<u64> = self
            .corpses
            .iter()
            .filter(|(_, corpse)| corpse.expires <= now)
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            self.corpses.remove(&id);
            events.push(Event::CorpseExpired(id));
        }
        Ok(events)
    }
    /// An attacked creature without a target turns on its attacker.
    fn provoke(&mut self, creature: u64, player: u64, now: u64, events: &mut Vec<Event>) {
        if let Some(actor) = self.actors.get_mut(&creature).filter(|actor| actor.health > 0 && actor.target.is_none()) {
            actor.target = Some(player);
            actor.returning = false;
            actor.next_attack = now.saturating_add(actor.rule.interval);
            events.push(Event::IdolVisual(creature, crate::combat_visual::IdolAction::Combat));
        }
    }

    /// Settle one hit by `player` on creature `target`: payment, rewards, quest
    /// credit, training loot (direct hits only), events and death. Returns false
    /// when the player cannot pay or accept the reward.
    #[allow(clippy::too_many_arguments)]
    fn land(
        &mut self,
        player: u64,
        target: u64,
        dealt: u32,
        cost: u32,
        direct: bool,
        now: u64,
        store: &mut Store,
        events: &mut Vec<Event>,
    ) -> io::Result<bool> {
        let Some(character) = store.characters.get(&player).filter(|c| !c.deleted) else {
            return Ok(false);
        };
        let actor = &self.actors[&target];
        let dealt = dealt.min(actor.health);
        if dealt == 0 {
            return Ok(true);
        }
        let killed = dealt == actor.health;
        let mut commands = Vec::new();
        if cost > 0 {
            commands.push(Command::SpendVigor(cost));
        }
        if killed && actor.rule.xp > 0 {
            commands.push(Command::AwardXp(actor.rule.xp));
        }
        // With loot enabled the gold waits in the corpse instead.
        if killed && actor.rule.gold > 0 && !self.training_loot {
            commands.push(Command::AwardGold(actor.rule.gold));
        }
        let candidate = commands
            .iter()
            .try_fold(character.progression.clone(), |state, command| state.apply(command));
        if candidate.is_err() {
            events.push(Event::Note {
                player,
                text: "Attack stopped: insufficient vigor or reward capacity.".into(),
            });
            return Ok(false);
        }
        {
            let account = character.account.clone();
            let previous = character.progression.clone();
            let previous_quests = character.quests.clone();
            if killed && !commands.is_empty() {
                store.encounter_reward(&account, player, actor.entity, &commands)?;
            } else if !commands.is_empty() {
                store.progression_batch(&account, player, &commands)?;
            }
            let rule = actor.rule;
            let loot = (killed && self.training_loot).then(|| self.roll_loot(&rule));
            if store.characters[&player].quests != previous_quests {
                events.push(Event::Quest {
                    player,
                    book: store.characters[&player].quests.clone(),
                });
            }
            let actor = self.actors.get_mut(&target).unwrap();
            actor.health -= dealt;
            if let Ok(dealt) = i32::try_from(dealt) {
                events.push(Event::Hit {
                    victim: target,
                    attacker: player,
                    health_change: -dealt,
                });
            }
            events.push(Event::Health {
                actor: target,
                health: actor.health,
            });
            events.push(Event::Owner {
                player,
                previous,
                state: store.characters[&player].progression.clone(),
            });
            events.push(Event::Note {
                player,
                text: if direct {
                    format!("You hit for {dealt} damage.")
                } else {
                    format!("Your lingering effect deals {dealt} damage.")
                },
            });
            if killed {
                actor.target = None;
                actor.moving = false;
                actor.respawn_at = Some(now.saturating_add(actor.rule.respawn));
                actor.remains = loot.map(|loot| (player, loot));
                events.push(Event::Note {
                    player,
                    text: if loot.is_some() {
                        format!("Creature defeated. Earned {} XP. Use its corpse to collect the loot.", actor.rule.xp)
                    } else {
                        format!("Creature defeated. Earned {} XP and {} gold.", actor.rule.xp, actor.rule.gold)
                    },
                });
                if actor.entity == crate::quests::TARGET_ENTITY {
                    events.push(Event::IdolVisual(
                        target,
                        crate::combat_visual::IdolAction::Death,
                    ));
                    // Death presentation grace; the corpse replaces it afterwards.
                    actor.despawn_at = Some(now.saturating_add(1500.min(actor.rule.respawn)));
                } else {
                    self.bury(target, now, events);
                }
                self.burns.retain(|burn| burn.target != target);
                let stopped = self
                    .attacks
                    .iter()
                    .filter(|(_, queued)| queued.target == target)
                    .map(|(&player, _)| player)
                    .collect::<Vec<_>>();
                for player in stopped {
                    self.stopped(player, events);
                }
            } else if actor.target.is_none() {
                actor.target = Some(player);
                actor.returning = false;
                actor.next_attack = now.saturating_add(actor.rule.interval);
                events.push(Event::IdolVisual(
                    target,
                    crate::combat_visual::IdolAction::Combat,
                ));
            }
        }
        Ok(true)
    }

    fn tick_burns(
        &mut self,
        now: u64,
        store: &mut Store,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        let due: Vec<usize> = (0..self.burns.len())
            .filter(|&index| self.burns[index].next <= now)
            .collect();
        for index in due.into_iter().rev() {
            let (source, target, per_tick) = {
                let burn = &self.burns[index];
                (burn.source, burn.target, burn.per_tick)
            };
            let alive = self.actors.get(&target).is_some_and(|actor| actor.health > 0);
            if alive {
                self.land(source, target, per_tick, 0, false, now, store, events)?;
            }
            if let Some(burn) = self.burns.get_mut(index) {
                burn.next = burn.next.saturating_add(burn.period);
            }
        }
        self.burns.retain(|burn| burn.next <= burn.until && self.actors.get(&burn.target).is_some_and(|a| a.health > 0));
        Ok(())
    }

    fn tick_mends(
        &mut self,
        now: u64,
        scene: &Scene,
        store: &mut Store,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        for mend in &mut self.mends {
            if mend.next > now {
                continue;
            }
            mend.next = mend.next.saturating_add(mend.period);
            let Some(character) = store
                .characters
                .get(&mend.player)
                .filter(|c| !c.deleted && c.progression.alive())
            else {
                mend.until = 0;
                continue;
            };
            if scene.player_placement(mend.player).is_none() {
                continue;
            }
            if mend.fx != 0 {
                events.push(Event::Fx {
                    target: mend.player,
                    fx: std::mem::take(&mut mend.fx),
                });
            }
            let level = character.progression.level();
            let mut commands = Vec::new();
            if mend.health > 0 && character.progression.health < level.health {
                commands.push(Command::Heal(mend.health));
            }
            if mend.vigor > 0 && character.progression.vigor < level.vigor {
                commands.push(Command::RecoverVigor(mend.vigor));
            }
            if commands.is_empty() {
                continue;
            }
            let account = character.account.clone();
            let previous = character.progression.clone();
            store.progression_batch(&account, mend.player, &commands)?;
            let state = store.characters[&mend.player].progression.clone();
            let healed = state.health.saturating_sub(previous.health);
            if healed > 0 {
                if let Ok(healed) = i32::try_from(healed) {
                    events.push(Event::Hit {
                        victim: mend.player,
                        attacker: mend.source,
                        health_change: healed,
                    });
                }
            }
            events.push(Event::Owner {
                player: mend.player,
                previous,
                state,
            });
        }
        self.mends.retain(|mend| mend.next <= mend.until);
        Ok(())
    }

    fn tick_creatures(
        &mut self,
        now: u64,
        scene: &Scene,
        store: &mut Store,
        active: &[u64],
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        let ids: Vec<u64> = self.actors.keys().copied().collect();
        for id in ids {
            let actor = self.actors.get_mut(&id).unwrap();
            if actor.health == 0 {
                if actor.despawn_at.is_some_and(|until| now >= until) {
                    actor.despawn_at = None;
                    self.bury(id, now, events);
                    continue;
                }
                if actor.respawn_at.is_some_and(|until| now >= until) {
                    actor.health = actor.rule.health;
                    actor.respawn_at = None;
                    actor.target = None;
                    actor.returning = false;
                    actor.moving = false;
                    events.push(Event::Respawn {
                        actor: id,
                        health: actor.health,
                    });
                }
                continue;
            }
            let Some(position) = scene.content_placement(id) else {
                continue;
            };
            let home = actor.home;
            let rule = actor.rule;
            // Alive, standing in this creature's cell and not past its leash.
            let pursuable = |player: u64| {
                store
                    .characters
                    .get(&player)
                    .is_some_and(|c| !c.deleted && c.progression.alive())
                    && scene.player_placement(player).is_some_and(|p| {
                        p.cell == position.cell
                            && distance(p.coordinates, home.coordinates) <= rule.homesick
                    })
            };
            if let Some(target) = actor.target.filter(|&target| !pursuable(target)) {
                let next = self
                    .attacks
                    .iter()
                    .find(|(player, attack)| attack.target == id && **player != target && pursuable(**player))
                    .map(|(&player, _)| player);
                let actor = self.actors.get_mut(&id).unwrap();
                actor.target = next;
                actor.next_attack = now.saturating_add(rule.interval);
                if next.is_none() {
                    actor.returning = true;
                    events.push(Event::IdolVisual(id, crate::combat_visual::IdolAction::Peace));
                    if actor.health != rule.health {
                        actor.health = rule.health;
                        events.push(Event::Health { actor: id, health: actor.health });
                    }
                }
            }
            let actor = self.actors.get_mut(&id).unwrap();
            if actor.target.is_none() && !actor.returning && rule.perception > 0.0 {
                let noticed = active
                    .iter()
                    .copied()
                    .filter(|&player| pursuable(player))
                    .filter_map(|player| {
                        let at = scene.player_placement(player)?;
                        let gap = distance(at.coordinates, position.coordinates);
                        (gap <= rule.perception).then_some((gap, player))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, player)| player);
                if let Some(player) = noticed {
                    actor.target = Some(player);
                    actor.next_attack = now.saturating_add(rule.interval);
                    events.push(Event::IdolVisual(id, crate::combat_visual::IdolAction::Combat));
                    events.push(Event::Note { player, text: "A creature attacks you!".into() });
                }
            }
            let chase = actor.target.and_then(|target| scene.player_placement(target));
            let goal = chase.or_else(|| actor.returning.then_some(home));
            if let Some(goal) = goal.filter(|_| now >= actor.next_move) {
                let stop = if chase.is_some() { (rule.range * 0.8).max(0.5) } else { 0.0 };
                let gap = distance(position.coordinates, goal.coordinates);
                let heading = combat::heading(position.coordinates, goal.coordinates).unwrap_or(position.heading);
                if gap > stop + 0.05 {
                    let step = (CHASE_SPEED * MOVE_MS as f32 / 1000.0).min(gap - stop);
                    let fraction = step / gap;
                    let coordinates = std::array::from_fn(|i| {
                        position.coordinates[i] + (goal.coordinates[i] - position.coordinates[i]) * fraction
                    });
                    actor.moving = true;
                    actor.next_move = now.saturating_add(MOVE_MS);
                    events.push(Event::Move {
                        actor: id,
                        placement: Placement { cell: position.cell, coordinates, heading },
                        motion: RUN_MOTION,
                    });
                } else {
                    if actor.moving {
                        actor.moving = false;
                        events.push(Event::Move {
                            actor: id,
                            placement: Placement { heading, ..position },
                            motion: STAND_MOTION,
                        });
                    }
                    if chase.is_none() {
                        actor.returning = false;
                    }
                }
            }
            let Some(target) = actor.target else {
                continue;
            };
            if now < actor.next_attack
                || !in_range(scene.player_placement(target), Some(position), rule.range)
            {
                continue;
            }
            actor.next_attack = now.saturating_add(rule.interval);
            events.push(Event::IdolVisual(id, crate::combat_visual::IdolAction::Swing));
            let character = &store.characters[&target];
            let level = character.progression.level();
            let defense = combat::player_defense(level.number, &character.skills, School::Melee);
            if !self.dice.roll(combat::hit_chance(rule.attack, defense)) {
                events.push(Event::Note { player: target, text: "The creature misses you.".into() });
                continue;
            }
            let account = character.account.clone();
            let previous = character.progression.clone();
            // Level-table ArmorRating, as Player.ArmorRating caps it in the 2012 server.
            let raw = combat::natural_damage(rule.damage, rule.variance, &mut self.dice);
            let dealt = combat::mitigate(raw, level.armor, &mut self.dice);
            store.progression_command(&account, target, Command::Damage(dealt))?;
            if let Ok(damage) = i32::try_from(dealt) {
                events.push(Event::Hit {
                    victim: target,
                    attacker: id,
                    health_change: -damage,
                });
            }
            let state = store.characters[&target].progression.clone();
            events.push(Event::Note {
                player: target,
                text: format!("Creature hits you for {dealt} damage."),
            });
            if !state.alive() {
                self.attacks.remove(&target);
                events.push(Event::Stop(target));
                events.push(Event::Note {
                    player: target,
                    text: "You have fallen. Use Resurrect to recover.".into(),
                });
            }
            events.push(Event::Owner {
                player: target,
                previous,
                state,
            });
        }
        Ok(())
    }

    fn tick_regen(
        &mut self,
        now: u64,
        scene: &Scene,
        store: &mut Store,
        active: impl Iterator<Item = u64>,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        self.regen.retain(|_, (_, until)| *until > now);
        if now >= self.next_regen {
            self.next_regen = now.saturating_add(self.regen_interval);
            let busy: BTreeSet<_> = self
                .attacks
                .keys()
                .copied()
                .chain(self.actors.values().filter_map(|actor| actor.target))
                .collect();
            for player in active {
                if scene.player_placement(player).is_none() || busy.contains(&player) {
                    continue;
                }
                let Some(character) = store
                    .characters
                    .get(&player)
                    .filter(|c| !c.deleted && c.progression.alive())
                else {
                    continue;
                };
                let state = &character.progression;
                let level = state.level();
                let (health_rate, vigor_rate) = self
                    .regen
                    .get(&player)
                    .map_or((1.0, 1.0), |(regen, _)| (regen.health, regen.vigor));
                let scaled = |base: u32, rate: f32| ((base as f32) * rate).round().max(0.0) as u32;
                let mut commands = Vec::new();
                let health = scaled(self.regen_health, health_rate);
                if health > 0 && state.health < level.health {
                    commands.push(Command::Heal(health));
                }
                let vigor = scaled(self.regen_vigor, vigor_rate);
                if vigor > 0 && state.vigor < level.vigor {
                    commands.push(Command::RecoverVigor(vigor));
                }
                if commands.is_empty() {
                    continue;
                }
                let account = character.account.clone();
                let previous = character.progression.clone();
                store.progression_batch(&account, player, &commands)?;
                events.push(Event::Owner {
                    player,
                    previous,
                    state: store.characters[&player].progression.clone(),
                });
            }
        }
        Ok(())
    }

    /// Restore a player's vitals: once on the next tick when `duration_ms` is 0,
    /// otherwise every period until it expires. Recasting refreshes the effect.
    pub fn mend(
        &mut self,
        source: u64,
        player: u64,
        over: Over,
        fx: u32,
        now: u64,
    ) -> io::Result<()> {
        if over.health == 0 && over.vigor == 0 {
            return Err(invalid("empty restoration"));
        }
        let period = over.period_ms.max(250);
        if over.duration_ms > 0 {
            self.mends.retain(|m| !(m.source == source && m.player == player && m.until > m.next));
        }
        if self.mends.len() >= MAX_TIMED {
            return Err(invalid("timed effect capacity"));
        }
        self.mends.push(Mend {
            source,
            player,
            health: over.health,
            vigor: over.vigor,
            period,
            next: now,
            until: now.saturating_add(over.duration_ms),
            fx,
        });
        Ok(())
    }

    /// Multiply a player's regeneration until the buff expires.
    pub fn boost_regen(&mut self, player: u64, regen: Regen, now: u64) -> io::Result<()> {
        if !(regen.health.is_finite() && regen.vigor.is_finite())
            || regen.health < 0.0
            || regen.vigor < 0.0
            || regen.health > 100.0
            || regen.vigor > 100.0
        {
            return Err(invalid("invalid regeneration buff"));
        }
        if self.regen.len() >= MAX_TIMED && !self.regen.contains_key(&player) {
            return Err(invalid("timed effect capacity"));
        }
        let (current, _) = self
            .regen
            .get(&player)
            .copied()
            .unwrap_or((Regen { health: 1.0, vigor: 1.0, duration_ms: 0 }, 0));
        let merged = Regen {
            health: if regen.health == 1.0 { current.health } else { regen.health },
            vigor: if regen.vigor == 1.0 { current.vigor } else { regen.vigor },
            duration_ms: regen.duration_ms,
        };
        self.regen
            .insert(player, (merged, now.saturating_add(regen.duration_ms)));
        Ok(())
    }
}
