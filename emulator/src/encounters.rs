//! Deterministic development encounters. Rules are explicit server-authored data.
//! This module does not claim original combat balance, collision or animation parity.
use crate::{
    content::Content,
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
}
struct Actor {
    entity: u32,
    rule: Rule,
    health: u32,
    target: Option<u64>,
    next_attack: u64,
    respawn_at: Option<u64>,
    despawn_at: Option<u64>,
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
    Item {
        player: u64,
        item: crate::inventory::Item,
        instance: u16,
    },
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
            if f.len() != 7 {
                return Err(invalid("creature combat field count"));
            }
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
            if entity >> 24 != 0x47
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
                gold: 2,
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
        };
        for line in include_str!("../content/creature-entities.txt").lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("pyrealium-") {
                continue;
            }
            let entity = u32::from_str_radix(line, 16).map_err(|_| invalid("creature entity"))?;
            rules.entry(entity).or_insert(starter);
        }
        let actors = content
            .objects()
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
        })
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
            let actor = &self.actors[&attack.target];
            let damage = attack
                .strike
                .map_or(self.damage, |strike| self.damage.saturating_add(strike.bonus));
            let dealt = damage
                .saturating_sub(actor.rule.armor)
                .max(1)
                .min(actor.health);
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
            if !self.land(player, attack.target, dealt, cost, true, now, scene, store, &mut events)? {
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
            if let Some(strike) = attack.strike {
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
        self.tick_burns(now, scene, store, &mut events)?;
        self.tick_mends(now, scene, store, &mut events)?;
        self.tick_creatures(now, scene, store, &mut events)?;
        self.tick_regen(now, scene, store, active, &mut events)?;
        Ok(events)
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
        scene: &Scene,
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
        if killed && actor.rule.gold > 0 {
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
            let loot = if direct && killed && self.training_loot {
                character
                    .inventory
                    .free_slot()
                    .filter(|_| character.inventory.dense_bags())
                    .and_then(|slot| {
                        store
                            .next_item_id()
                            .ok()
                            .map(|id| crate::item_services::potion(id, slot))
                    })
                    .and_then(|item| {
                        scene
                            .admit_inventory_grant(player, item.id)
                            .ok()
                            .map(|instance| (item, instance))
                    })
            } else {
                None
            };
            if let Some((item, instance)) = &loot {
                store.encounter_transaction(
                    &account,
                    player,
                    actor.entity,
                    &[crate::inventory::Command::Grant(item.clone())],
                    &commands,
                )?;
                events.push(Event::Item {
                    player,
                    item: item.clone(),
                    instance: *instance,
                });
            } else if killed {
                store.encounter_reward(&account, player, actor.entity, &commands)?;
            } else if !commands.is_empty() {
                store.progression_batch(&account, player, &commands)?;
            }
            if killed && self.training_loot && loot.is_none() {
                events.push(Event::Note {player,text:"No training potion was granted: make room in your bag. XP, gold and quest credit are saved.".into()});
            }
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
                actor.respawn_at = Some(now.saturating_add(actor.rule.respawn));
                events.push(Event::Note {
                    player,
                    text: format!(
                        "Creature defeated. Earned {} XP and {} gold.",
                        actor.rule.xp, actor.rule.gold
                    ),
                });
                if actor.entity == crate::quests::TARGET_ENTITY {
                    events.push(Event::IdolVisual(
                        target,
                        crate::combat_visual::IdolAction::Death,
                    ));
                    // Authored presentation grace; rewards and attack authority
                    // are already settled. This is not a lootable corpse.
                    actor.despawn_at = Some(now.saturating_add(1500.min(actor.rule.respawn)));
                } else {
                    events.push(Event::Despawn(target));
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
        scene: &Scene,
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
                self.land(source, target, per_tick, 0, false, now, scene, store, events)?;
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
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        for (&id, actor) in &mut self.actors {
            if actor.health == 0 {
                if actor.despawn_at.is_some_and(|until| now >= until) {
                    actor.despawn_at = None;
                    events.push(Event::Despawn(id));
                }
                if actor.respawn_at.is_some_and(|until| now >= until) {
                    actor.health = actor.rule.health;
                    actor.respawn_at = None;
                    actor.target = None;
                    events.push(Event::Respawn {
                        actor: id,
                        health: actor.health,
                    });
                }
                continue;
            }
            let Some(mut target) = actor.target else {
                continue;
            };
            let eligible = |player| {
                store
                    .characters
                    .get(&player)
                    .is_some_and(|c| !c.deleted && c.progression.alive())
                    && in_range(
                        scene.player_placement(player),
                        scene.content_placement(id),
                        actor.rule.range,
                    )
            };
            if !eligible(target) {
                if let Some((&other, _)) = self
                    .attacks
                    .iter()
                    .find(|(player, attack)| attack.target == id && eligible(**player))
                {
                    target = other;
                    actor.target = Some(other);
                    actor.next_attack = now.saturating_add(actor.rule.interval);
                } else {
                    actor.target = None;
                    events.push(Event::IdolVisual(
                        id,
                        crate::combat_visual::IdolAction::Peace,
                    ));
                    if actor.health != actor.rule.health {
                        actor.health = actor.rule.health;
                        events.push(Event::Health {
                            actor: id,
                            health: actor.health,
                        });
                    }
                    continue;
                }
            }
            if now < actor.next_attack {
                continue;
            }
            let account = store.characters[&target].account.clone();
            let previous = store.characters[&target].progression.clone();
            // ArmorRating from Pyreal_DB_Insert_PlayerLevelData.sql. At least 1
            // damage remains so a hit is still a hit.
            let armor = previous.level().armor;
            let dealt = actor.rule.damage.saturating_sub(armor).max(1);
            store.progression_command(&account, target, Command::Damage(dealt))?;
            actor.next_attack = now.saturating_add(actor.rule.interval);
            events.push(Event::IdolVisual(
                id,
                crate::combat_visual::IdolAction::Swing,
            ));
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
