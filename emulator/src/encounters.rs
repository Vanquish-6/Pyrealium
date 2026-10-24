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
}
pub enum Event {
    IdolVisual(u64, crate::combat_visual::IdolAction),
    Swing(u64),
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
        if actors.is_empty() || actors.len() > 512 {
            return Err(invalid("encounters require 1..512 matching placed actors"));
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
            if !available
                || !alive
                || !in_range(
                    scene.player_placement(player),
                    scene.content_placement(attack.target),
                    self.range,
                )
            {
                self.stopped(player, &mut events);
                continue;
            }
            let actor = &self.actors[&attack.target];
            let dealt = self.damage.min(actor.health);
            let killed = dealt == actor.health;
            let mut commands = vec![Command::SpendVigor(self.cost)];
            if killed && actor.rule.xp > 0 {
                commands.push(Command::AwardXp(actor.rule.xp));
            }
            if killed && actor.rule.gold > 0 {
                commands.push(Command::AwardGold(actor.rule.gold));
            }
            let character = &store.characters[&player];
            let candidate = commands
                .iter()
                .try_fold(character.progression.clone(), |state, command| {
                    state.apply(command)
                });
            if candidate.is_err() {
                events.push(Event::Note {
                    player,
                    text: "Attack stopped: insufficient vigor or reward capacity.".into(),
                });
                self.stopped(player, &mut events);
                continue;
            }
            let account = character.account.clone();
            let previous = character.progression.clone();
            let previous_quests = character.quests.clone();
            let loot = if killed && self.training_loot {
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
            } else {
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
            let until = now.saturating_add(self.interval);
            self.cooldowns.insert(player, until);
            self.attacks.get_mut(&player).unwrap().next = until;
            let actor = self.actors.get_mut(&attack.target).unwrap();
            actor.health -= dealt;
            events.push(Event::Swing(player));
            events.push(Event::Health {
                actor: attack.target,
                health: actor.health,
            });
            events.push(Event::Owner {
                player,
                previous,
                state: store.characters[&player].progression.clone(),
            });
            events.push(Event::Note {
                player,
                text: format!("You hit for {dealt} damage."),
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
                        attack.target,
                        crate::combat_visual::IdolAction::Death,
                    ));
                    // Authored presentation grace; rewards and attack authority
                    // are already settled. This is not a lootable corpse.
                    actor.despawn_at = Some(now.saturating_add(1500.min(actor.rule.respawn)));
                } else {
                    events.push(Event::Despawn(attack.target));
                }
                let stopped = self
                    .attacks
                    .iter()
                    .filter(|(_, queued)| queued.target == attack.target)
                    .map(|(&player, _)| player)
                    .collect::<Vec<_>>();
                for player in stopped {
                    self.stopped(player, &mut events);
                }
            } else if actor.target.is_none() {
                actor.target = Some(player);
                actor.next_attack = now.saturating_add(actor.rule.interval);
                events.push(Event::IdolVisual(
                    attack.target,
                    crate::combat_visual::IdolAction::Combat,
                ));
            }
            if !attack.repeat {
                self.attacks.remove(&player);
            }
        }
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
            store.progression_command(&account, target, Command::Damage(actor.rule.damage))?;
            actor.next_attack = now.saturating_add(actor.rule.interval);
            events.push(Event::IdolVisual(
                id,
                crate::combat_visual::IdolAction::Swing,
            ));
            let state = store.characters[&target].progression.clone();
            events.push(Event::Note {
                player: target,
                text: format!("Creature hits you for {} damage.", actor.rule.damage),
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
                let mut commands = Vec::new();
                if self.regen_health > 0 && state.health < level.health {
                    commands.push(Command::Heal(self.regen_health));
                }
                if self.regen_vigor > 0 && state.vigor < level.vigor {
                    commands.push(Command::RecoverVigor(self.regen_vigor));
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
        Ok(events)
    }
}
