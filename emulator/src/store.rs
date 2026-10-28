//! Transactional local character journal with checked progression compaction.
//! A write is acknowledged only after fsync.
//! Corruption/torn tails stop startup rather than silently discarding characters.
use crate::movement::Placement;
use crate::world::CharacterCreationRequest;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PHYSICAL_VALUES: usize = 256;
mod compaction;

#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub id: u64,
    pub account: String,
    pub name: String,
    /// Exact ordered client appearance values; None for legacy CLI-created characters.
    pub creation: Option<CharacterCreationRequest>,
    pub deleted: bool,
    /// Durable avatar lifetime; never reused by reconnecting transports.
    pub instance: u16,
    pub ui: Option<crate::preferences::UiSettings>,
    pub hotbar: crate::preferences::Hotbar,
    pub gameplay_options: Option<crate::preferences::GameplayOptions>,
    pub inventory: crate::inventory::Inventory,
    pub progression: crate::progression::Progression,
    pub skills: crate::skills::Book,
    pub quests: crate::quests::Book,
    pub training_kit: bool,
    pub training_supplies: bool,
    pub starter_gear: bool,
    pub binding: Option<Placement>,
}

pub struct Store {
    file: File,
    journal_path: PathBuf,
    compact_after: u64,
    lock: PathBuf,
    pub characters: BTreeMap<u64, Character>,
    poisoned: bool,
    positions_path: PathBuf,
    positions: BTreeMap<u64, Placement>,
    positions_dirty: bool,
    revision: u64,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}
fn hash(bytes: &[u8]) -> u32 {
    bytes.iter().fold(2166136261u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(16777619)
    })
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ' || b == b'-')
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let lock = dir.join("server.lock");
        let mut guard = OpenOptions::new().write(true).create_new(true).open(&lock)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot lock {}: {e}; after a crash, verify no server is running before removing this lock", lock.display())))?;
        let result = (|| {
            writeln!(guard, "{}", std::process::id())?;
            guard.sync_all()?;
            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(dir.join("characters.journal"))?;
            if file.metadata()?.len() > MAX_JOURNAL_BYTES {
                return Err(invalid("journal exceeds 16 MiB limit"));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let journal = decode_journal_state(&bytes)?;
            let characters = journal.characters;
            let positions_path = dir.join("positions.snapshot");
            let (position_revision, mut positions) = match File::open(&positions_path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    decode_position_snapshot(&bytes, &characters)?
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (0, BTreeMap::new()),
                Err(e) => return Err(e),
            };
            if position_revision > journal.revision {
                return Err(invalid("position snapshot is newer than character journal"));
            }
            let mut positions_dirty = false;
            for (id, (revision, placement)) in journal.relocations {
                if revision > position_revision {
                    positions.insert(id, placement);
                    positions_dirty = true;
                }
            }
            Ok(Self {
                file,
                journal_path: dir.join("characters.journal"),
                compact_after: MAX_JOURNAL_BYTES / 2,
                lock: lock.clone(),
                characters,
                poisoned: false,
                positions_path,
                positions,
                positions_dirty,
                revision: journal.revision,
            })
        })();
        if result.is_err() {
            drop(guard);
            let _ = fs::remove_file(lock);
        }
        result
    }
    pub fn create(&mut self, account: &str, name: &str) -> io::Result<Character> {
        self.create_record(account, name, None)
    }
    pub fn placement(&self, id: u64) -> Option<Placement> {
        self.positions.get(&id).copied()
    }
    pub fn next_item_id(&self) -> io::Result<u64> {
        self.characters
            .values()
            .map(|c| c.inventory.high_water())
            .max()
            .unwrap_or(crate::inventory::ITEM_NAMESPACE)
            .checked_add(1)
            .filter(|id| id >> 48 == crate::inventory::ITEM_NAMESPACE >> 48)
            .ok_or_else(|| invalid("item identity exhausted"))
    }
    pub fn grant_starter_gear(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self.characters.get(&id)
            .filter(|character| character.account == account && !character.deleted)
            .ok_or_else(|| invalid("starter gear character not owned"))?;
        if character.starter_gear { return Ok(false); }
        let request = character.creation.as_ref().ok_or_else(|| invalid("Starter gear needs a saved creation profile."))?;
        let first = self.next_item_id()?;
        let next = crate::starter_gear::candidate(request, &character.inventory, first)?;
        self.append_record(&format!("t4\t{id}\t{first}"))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.inventory = next;
        character.starter_gear = true;
        character.training_kit = true;
        Ok(true)
    }
    pub fn grant_training_kit(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("training kit character not owned"))?;
        if character.training_kit {
            return Ok(false);
        }
        let Some(race) = character
            .creation
            .as_ref()
            .filter(|request| crate::appearance::body(request.race, request.sex).is_some())
            .map(|request| request.race)
        else {
            return Err(invalid("training kit requires a playable character profile"));
        };
        let slot = character
            .inventory
            .free_slot()
            .ok_or_else(|| invalid("training kit requires a free bag slot"))?;
        let item = self.next_item_id()?;
        let next = character
            .inventory
            .apply(&crate::inventory::Command::Grant(
                crate::training_kit::item(item, slot),
            ))
            .map_err(|_| invalid("invalid training kit grant"))?;
        let human = race == crate::appearance::HUMAN;
        let skills = if human {
            character
                .skills
                .with_training_attack(&character.progression, race)
                .map_err(io::Error::other)?
        } else {
            character.skills.clone()
        };
        // One fsync publishes the item, basic attack and once-per-character marker.
        // t1 is the item-only grant; other races get their attacks from k5.
        self.append_record(&format!("{}\t{id}\t{item}", if human { "t2" } else { "t1" }))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.inventory = next;
        character.skills = skills;
        character.training_kit = true;
        Ok(true)
    }
    pub fn grant_training_supplies(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.training_kit)
            .ok_or_else(|| {
                invalid("training supplies character not owned or equipped for training")
            })?;
        if character.training_supplies {
            return Ok(false);
        }
        if !character.progression.alive() {
            return Err(invalid("training supplies require a living character"));
        }
        let first = self.next_item_id()?;
        let next = crate::training_kit::supply_inventory(&character.inventory, first)
            .map_err(io::Error::other)?;
        // One durable record grants all five items and the once-only marker.
        self.append_record(&format!("t3\t{id}\t{first}"))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.inventory = next;
        character.training_supplies = true;
        Ok(true)
    }
    pub fn ensure_training_attack(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.training_kit)
            .ok_or_else(|| invalid("training attack requires an owned training character"))?;
        if character
            .skills
            .get(crate::skills::TRAINING_ATTACK)
            .is_some()
            || character
                .creation
                .as_ref()
                .is_some_and(|request| request.race != crate::appearance::HUMAN)
        {
            return Ok(false);
        }
        let skills = character
            .skills
            .with_training_attack(
                &character.progression,
                character
                    .creation
                    .as_ref()
                    .map_or(1, |request| request.race),
            )
            .map_err(io::Error::other)?;
        self.append_record(&format!("k4\t{id}"))?;
        self.characters.get_mut(&id).unwrap().skills = skills;
        Ok(true)
    }
    pub fn ensure_starter_skills(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("starter skills require an owned character"))?;
        let Some((race, sex)) = character
            .creation
            .as_ref()
            .map(|request| (request.race, request.sex))
            .filter(|&(race, sex)| crate::appearance::body(race, sex).is_some())
        else {
            return Ok(false);
        };
        if character.skills.has_starter_skills(race, sex) || !character.progression.alive() {
            return Ok(false);
        }
        let skills = character
            .skills
            .with_starter_skills(&character.progression, race, sex)
            .map_err(io::Error::other)?;
        self.append_record(&format!("k5\t{id}"))?;
        self.characters.get_mut(&id).unwrap().skills = skills;
        Ok(true)
    }
    pub fn bind_lifestone(
        &mut self,
        account: &str,
        id: u64,
        placement: Placement,
    ) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.progression.alive())
            .ok_or_else(|| invalid("binding requires an owned living character"))?;
        if !placement.valid() {
            return Err(invalid("invalid lifestone bind point"));
        }
        let skills = character
            .skills
            .with_lifestone_recall(
                &character.progression,
                character
                    .creation
                    .as_ref()
                    .map_or(1, |request| request.race),
            )
            .map_err(io::Error::other)?;
        let quests = if character.quests.journey_phase() == 1 {
            character.quests.apply(&crate::quests::Command::BoundLifestone).map_err(io::Error::other)?
        } else { character.quests.clone() };
        if quests == character.quests && character.binding == Some(placement)
            && character
                .skills
                .get(crate::skills::LIFESTONE_RECALL)
                .is_some()
        {
            return Ok(());
        }
        let revision = if quests != character.quests { "l3" } else { "l2" };
        self.append_record(&format!("{revision}\t{id}\t{}", encode_placement(placement)))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.binding = Some(placement);
        character.skills = skills;
        character.quests = quests;
        Ok(())
    }
    pub fn ensure_lifestone_recall(&mut self, account: &str, id: u64) -> io::Result<bool> {
        let character = self
            .characters
            .get(&id)
            .filter(|character| {
                character.account == account && !character.deleted && character.binding.is_some()
            })
            .ok_or_else(|| invalid("recall grant requires an owned bound character"))?;
        if character
            .skills
            .get(crate::skills::LIFESTONE_RECALL)
            .is_some()
        {
            return Ok(false);
        }
        let skills = character
            .skills
            .with_lifestone_recall(
                &character.progression,
                character
                    .creation
                    .as_ref()
                    .map_or(1, |request| request.race),
            )
            .map_err(io::Error::other)?;
        self.append_record(&format!("k3\t{id}"))?;
        self.characters.get_mut(&id).unwrap().skills = skills;
        Ok(true)
    }
    /// Destination must have passed world/cell validation before this transaction.
    pub fn relocate(&mut self, account: &str, id: u64, placement: Placement) -> io::Result<()> {
        if !placement.valid()
            || !self.characters.get(&id).is_some_and(|character| {
                character.account == account && !character.deleted && character.progression.alive()
            })
        {
            return Err(invalid(
                "relocation requires an owned living character and valid placement",
            ));
        }
        self.append_record(&format!("p2\t{id}\t{}", encode_placement(placement)))?;
        self.positions.insert(id, placement);
        self.positions_dirty = true;
        Ok(())
    }
    pub fn relocate_training_portal(&mut self, account: &str, id: u64, placement: Placement, portal: u64) -> io::Result<()> {
        let character = self.characters.get(&id).filter(|c| c.account == account && !c.deleted && c.progression.alive()).ok_or_else(|| invalid("Travel requires an owned living character"))?;
        if character.quests.journey_phase() != 2 || !crate::quests::is_training_portal(portal) {
            return self.relocate(account, id, placement);
        }
        if !placement.valid() { return Err(invalid("Invalid training portal destination")); }
        let quests = character.quests.apply(&crate::quests::Command::Travelled).map_err(io::Error::other)?;
        self.append_record(&format!("p3\t{id}\t{portal:016x}\t{}", encode_placement(placement)))?;
        self.characters.get_mut(&id).unwrap().quests = quests;
        self.positions.insert(id, placement); self.positions_dirty = true;
        Ok(())
    }
    /// Binding must have passed world/cell validation before this transaction.
    pub fn return_to_binding(
        &mut self,
        account: &str,
        id: u64,
        resurrect: bool,
    ) -> io::Result<Placement> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("recall character not owned"))?;
        let placement = character
            .binding
            .ok_or_else(|| invalid("character is not bound to a lifestone"))?;
        let next = if resurrect {
            character
                .progression
                .apply(&crate::progression::Command::Resurrect)
                .map_err(|_| invalid("resurrection requires a dead character"))?
        } else {
            if !character.progression.alive() {
                return Err(invalid("recall requires a living character"));
            }
            character.progression.clone()
        };
        self.append_record(&format!(
            "{}\t{id}\t{}",
            if resurrect { "r2" } else { "p1" },
            encode_placement(placement)
        ))?;
        self.characters.get_mut(&id).unwrap().progression = next;
        self.positions.insert(id, placement);
        self.positions_dirty = true;
        Ok(placement)
    }
    pub fn progression_command(
        &mut self,
        account: &str,
        id: u64,
        command: crate::progression::Command,
    ) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("progression character not owned"))?;
        let next = character
            .progression
            .apply(&command)
            .map_err(|error| invalid(&error.to_string()))?;
        if next == character.progression {
            return Ok(());
        }
        self.append_record(&format!("g1\t{id}\t{}", command.encode()))?;
        self.characters.get_mut(&id).unwrap().progression = next;
        Ok(())
    }
    pub fn raise_skill(&mut self, account: &str, id: u64, skill: u32) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|character| character.account == account && !character.deleted)
            .ok_or_else(|| invalid("skill advancement character not owned"))?;
        let race = character
            .creation
            .as_ref()
            .map_or(1, |request| request.race);
        let (skills, progression) = character
            .skills
            .advance(skill, &character.progression, race)
            .map_err(io::Error::other)?;
        let cost = character
            .progression
            .available_xp
            .checked_sub(progression.available_xp)
            .filter(|cost| *cost > 0)
            .ok_or_else(|| invalid("invalid advancement cost"))?;
        self.append_record(&format!("k2\t{id}\t{skill:x}\t{cost}"))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.skills = skills;
        character.progression = progression;
        Ok(())
    }
    pub fn skill_command(
        &mut self,
        account: &str,
        id: u64,
        command: crate::skills::Command,
    ) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("skill character not owned"))?;
        let race = character
            .creation
            .as_ref()
            .map_or(1, |request| request.race);
        let next = character
            .skills
            .apply(command, &character.progression, race)
            .map_err(io::Error::other)?;
        self.append_record(&format!("k1\t{id}\t{}", command.encode()))?;
        self.characters.get_mut(&id).unwrap().skills = next;
        Ok(())
    }
    pub fn progression_batch(
        &mut self,
        account: &str,
        id: u64,
        commands: &[crate::progression::Command],
    ) -> io::Result<()> {
        if commands.is_empty() || commands.len() > 8 {
            return Err(invalid("progression batch size"));
        }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("progression character not owned"))?;
        let mut next = character.progression.clone();
        for command in commands {
            next = next
                .apply(command)
                .map_err(|error| invalid(&error.to_string()))?;
        }
        if next == character.progression {
            return Ok(());
        }
        let encoded = commands
            .iter()
            .map(crate::progression::Command::encode)
            .collect::<Vec<_>>()
            .join(";");
        self.append_record(&format!("g2\t{id}\t{encoded}"))?;
        self.characters.get_mut(&id).unwrap().progression = next;
        Ok(())
    }
    pub fn quest_command(
        &mut self,
        account: &str,
        id: u64,
        command: crate::quests::Command,
    ) -> io::Result<bool> {
        if matches!(command, crate::quests::Command::Kill(_) | crate::quests::Command::ClaimSupplies | crate::quests::Command::CraftedPotion | crate::quests::Command::BoundLifestone | crate::quests::Command::Travelled) {
            return Err(invalid("quest completion requires its authoritative reward transaction"));
        }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.progression.alive())
            .ok_or_else(|| invalid("quest requires an owned living character"))?;
        let quests = character.quests.apply(&command).map_err(io::Error::other)?;
        if quests == character.quests {
            return Ok(false);
        }
        let mut progression = character.progression.clone();
        for reward in crate::quests::rewards(&command) {
            progression = progression.apply(&reward).map_err(io::Error::other)?;
        }
        let revision = match command {
            crate::quests::Command::AcceptSupplies => "q3",
            crate::quests::Command::AcceptPotion | crate::quests::Command::ClaimPotion | crate::quests::Command::AcceptJourney | crate::quests::Command::ClaimJourney => "q4",
            crate::quests::Command::AcceptSkill | crate::quests::Command::UsedSkill | crate::quests::Command::ClaimSkill => "q5",
            crate::quests::Command::AcceptHunt | crate::quests::Command::ClaimHunt => "q6",
            _ => "q1",
        };
        self.append_record(&format!("{revision}\t{id}\t{}", command.encode()))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.quests = quests;
        character.progression = progression;
        Ok(true)
    }
    /// Persist the objective with the normal payout before publishing death.
    pub fn encounter_reward(
        &mut self,
        account: &str,
        id: u64,
        entity: u32,
        commands: &[crate::progression::Command],
    ) -> io::Result<()> {
        if commands.is_empty() || commands.len() > 8 {
            return Err(invalid("encounter reward batch size"));
        }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.progression.alive())
            .ok_or_else(|| invalid("encounter reward character unavailable"))?;
        let quests = character.quests.record_kill(entity);
        if quests == character.quests {
            return self.progression_batch(account, id, commands);
        }
        let mut progression = character.progression.clone();
        for command in commands {
            progression = progression.apply(command).map_err(io::Error::other)?;
        }
        self.append_record(&format!(
            "q2\t{id}\t{entity:08x}\t{}",
            crate::player_transaction::encode_changes(commands)
        ))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.quests = quests;
        character.progression = progression;
        Ok(())
    }
    pub fn encounter_transaction(
        &mut self,
        account: &str,
        id: u64,
        entity: u32,
        items: &[crate::inventory::Command],
        changes: &[crate::progression::Command],
    ) -> io::Result<()> {
        if entity >> 24 != 0x47 || items.is_empty() || changes.is_empty() {
            return Err(invalid("invalid item encounter reward"));
        }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted && c.progression.alive())
            .ok_or_else(|| invalid("encounter transaction owner unavailable"))?;
        let first = if items.iter().any(|command| command.allocated_id().is_some()) {
            Some(self.next_item_id()?)
        } else {
            None
        };
        let (inventory, progression) = crate::player_transaction::candidate(
            &character.inventory,
            &character.progression,
            items,
            changes,
            first,
        )?;
        let quests = character.quests.record_kill(entity);
        self.append_record(&format!(
            "a2\t{id}\t{entity:08x}\t{}\t{}",
            crate::player_transaction::encode_items(items),
            crate::player_transaction::encode_changes(changes)
        ))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.inventory = inventory;
        character.progression = progression;
        character.quests = quests;
        Ok(())
    }
    /// Items and payment/rewards become visible only after one durable record.
    pub fn player_transaction(
        &mut self,
        account: &str,
        id: u64,
        items: &[crate::inventory::Command],
        changes: &[crate::progression::Command],
    ) -> io::Result<()> {
        self.player_transaction_quest(account, id, items, changes, false, false)
    }
    pub fn player_transaction_quest(
        &mut self, account: &str, id: u64,
        items: &[crate::inventory::Command], changes: &[crate::progression::Command],
        supplies: bool, practice: bool,
    ) -> io::Result<()> {
        if supplies && practice { return Err(invalid("Multiple quest exchanges in one transaction")); }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("transaction character not owned"))?;
        let first = if items.iter().any(|command| command.allocated_id().is_some()) {
            Some(self.next_item_id()?)
        } else {
            None
        };
        let (inventory, progression) = crate::player_transaction::candidate(
            &character.inventory,
            &character.progression,
            items,
            changes,
            first,
        )?;
        if inventory == character.inventory && progression == character.progression {
            return Ok(());
        }
        let quests = if supplies {
            if !character.progression.alive() { return Err(invalid("Supply exchange requires a living player")); }
            crate::quests::validate_supplies_exchange(&character.quests, &character.inventory, items, changes, first.ok_or_else(|| invalid("Missing supply output identity"))?)?
        } else if practice {
            if !character.progression.alive() { return Err(invalid("Crafting requires a living owner")); }
            crate::quests::validate_practice_craft(&character.quests, &character.inventory, items, changes, first.ok_or_else(|| invalid("Missing craft output identity"))?)?
        } else { character.quests.clone() };
        let revision = if supplies { "a2" } else if practice { "a3" } else { "a1" };
        self.append_record(&format!(
            "{revision}\t{id}\t{}\t{}",
            crate::player_transaction::encode_items(items),
            crate::player_transaction::encode_changes(changes)
        ))?;
        let character = self.characters.get_mut(&id).unwrap();
        character.inventory = inventory;
        character.progression = progression;
        character.quests = quests;
        Ok(())
    }
    pub fn inventory_command(
        &mut self,
        account: &str,
        id: u64,
        command: crate::inventory::Command,
    ) -> io::Result<()> {
        if command
            .allocated_id()
            .is_some_and(|allocated| self.next_item_id().ok() != Some(allocated))
        {
            return Err(invalid("nonsequential item identity"));
        }
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("inventory character not owned"))?;
        let next = character
            .inventory
            .apply(&command)
            .map_err(|error| invalid(&error.to_string()))?;
        if next == character.inventory {
            return Ok(());
        }
        self.append_record(&format!("i1\t{id}\t{}", command.encode()))?;
        self.characters.get_mut(&id).unwrap().inventory = next;
        Ok(())
    }
    pub fn update_ui(
        &mut self,
        account: &str,
        id: u64,
        patch: crate::preferences::UiSettings,
    ) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("UI settings character not owned"))?;
        let settings = match &character.ui {
            Some(previous) => previous
                .merge(&patch)
                .map_err(|_| invalid("UI settings limit exceeded"))?,
            None => patch,
        };
        if character.ui.as_ref() == Some(&settings) {
            return Ok(());
        }
        let bytes = settings.encode();
        let mut hex = String::with_capacity(bytes.len() * 2);
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            hex.push(DIGITS[(byte >> 4) as usize] as char);
            hex.push(DIGITS[(byte & 15) as usize] as char);
        }
        self.append_record(&format!("u1\t{id}\t{hex}"))?;
        self.characters.get_mut(&id).unwrap().ui = Some(settings);
        Ok(())
    }
    pub fn update_gameplay_options(
        &mut self,
        account: &str,
        id: u64,
        options: crate::preferences::GameplayOptions,
    ) -> io::Result<()> {
        let character = self.characters.get(&id)
            .filter(|character| character.account == account && !character.deleted)
            .ok_or_else(|| invalid("gameplay settings character not owned"))?;
        if character.gameplay_options.as_ref() == Some(&options) { return Ok(()); }
        let hex = options.encode().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        self.append_record(&format!("o1\t{id}\t{hex}"))?;
        self.characters.get_mut(&id).unwrap().gameplay_options = Some(options);
        Ok(())
    }
    pub fn update_hotbar(
        &mut self,
        account: &str,
        id: u64,
        update: crate::preferences::HotbarUpdate,
    ) -> io::Result<()> {
        let character = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("hotbar character not owned"))?;
        let next = character
            .hotbar
            .apply(&update)
            .map_err(|_| invalid("hotbar size limit exceeded"))?;
        if next == character.hotbar {
            return Ok(());
        }
        let bytes = update
            .encode()
            .map_err(|_| invalid("invalid hotbar update"))?;
        let mut hex = String::with_capacity(bytes.len() * 2);
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            hex.push(DIGITS[(byte >> 4) as usize] as char);
            hex.push(DIGITS[(byte & 15) as usize] as char);
        }
        self.append_record(&format!("h1\t{id}\t{hex}"))?;
        self.characters.get_mut(&id).unwrap().hotbar = next;
        Ok(())
    }
    pub fn set_placement(&mut self, id: u64, placement: Placement) -> io::Result<()> {
        if self.characters.get(&id).is_none_or(|c| c.deleted) || !placement.valid() {
            return Err(invalid("invalid character placement"));
        }
        if self.positions.get(&id) != Some(&placement) {
            self.positions.insert(id, placement);
            self.positions_dirty = true;
        }
        Ok(())
    }
    /// One bounded snapshot per checkpoint, not an fsync for every movement packet.
    /// The old snapshot remains authoritative until the flushed new file is renamed.
    pub fn flush_positions(&mut self) -> io::Result<()> {
        if !self.positions_dirty {
            return Ok(());
        }
        let size = 24usize
            .checked_add(
                self.positions
                    .len()
                    .checked_mul(25)
                    .ok_or_else(|| invalid("too many placements"))?,
            )
            .filter(|size| *size <= 1024 * 1024)
            .ok_or_else(|| invalid("placement snapshot limit"))?;
        let mut bytes = Vec::with_capacity(size);
        bytes.extend(b"AC2POS02");
        bytes.extend(self.revision.to_le_bytes());
        bytes.extend((self.positions.len() as u32).to_le_bytes());
        for (id, placement) in &self.positions {
            bytes.extend(id.to_le_bytes());
            bytes.extend(placement.cell.to_le_bytes());
            for value in placement.coordinates {
                bytes.extend(value.to_le_bytes());
            }
            bytes.push(placement.heading);
        }
        bytes.extend(hash(&bytes).to_le_bytes());
        let temporary = self.positions_path.with_extension("snapshot.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &self.positions_path)?;
        self.positions_dirty = false;
        Ok(())
    }
    /// Persists structurally valid input. The caller must validate entity/world authority.
    pub fn create_native(
        &mut self,
        account: &str,
        request: &CharacterCreationRequest,
    ) -> io::Result<Character> {
        self.create_record(account, &request.name, Some(request))
    }
    fn create_record(
        &mut self,
        account: &str,
        name: &str,
        creation: Option<&CharacterCreationRequest>,
    ) -> io::Result<Character> {
        if self.poisoned {
            return Err(invalid(
                "journal write failed previously; restart and inspect journal",
            ));
        }
        if !valid_name(account) || !valid_name(name) {
            return Err(invalid(
                "account and character names must contain 1-32 ASCII letters, digits, spaces, hyphens or underscores",
            ));
        }
        if self
            .characters
            .values()
            .any(|c| c.name.eq_ignore_ascii_case(name))
        {
            return Err(invalid("character name already exists"));
        }
        if self
            .characters
            .values()
            .filter(|c| c.account == account && !c.deleted)
            .count()
            >= 5
        {
            return Err(invalid("local character limit is five"));
        }
        let id = self
            .characters
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("character IDs exhausted"))?;
        let c = Character {
            id,
            account: account.into(),
            name: name.into(),
            creation: creation.cloned(),
            deleted: false,
            instance: 0,
            ui: None,
            gameplay_options: None,
            hotbar: crate::preferences::Hotbar::default(),
            inventory: crate::inventory::Inventory::default(),
            progression: crate::progression::Progression::default(),
            skills: crate::skills::Book::default(),
            quests: crate::quests::Book::default(),
            training_kit: false,
            training_supplies: false,
            starter_gear: false,
            binding: None,
        };
        let line = match creation {
            None => format!("v1\t{}\t{}\t{}", c.id, c.account, c.name),
            Some(request) => format!(
                "v2\t{}\t{}\t{}\t{}",
                c.id,
                c.account,
                c.name,
                encode_creation(request)?
            ),
        };
        self.append_record(&line)?;
        self.characters.insert(id, c.clone());
        Ok(c)
    }
    pub fn delete(&mut self, account: &str, id: u64) -> io::Result<()> {
        if !self
            .characters
            .get(&id)
            .is_some_and(|c| c.account == account && !c.deleted)
        {
            return Err(invalid("character not owned or already deleted"));
        }
        self.append_record(&format!("d1\t{id}"))?;
        self.characters.get_mut(&id).unwrap().deleted = true;
        Ok(())
    }
    pub fn restore(&mut self, account: &str, id: u64) -> io::Result<()> {
        if !self
            .characters
            .get(&id)
            .is_some_and(|c| c.account == account && c.deleted)
        {
            return Err(invalid("deleted character not owned"));
        }
        if self
            .characters
            .values()
            .filter(|c| c.account == account && !c.deleted)
            .count()
            >= 5
        {
            return Err(invalid("local character limit is five"));
        }
        self.append_record(&format!("r1\t{id}"))?;
        self.characters.get_mut(&id).unwrap().deleted = false;
        Ok(())
    }
    pub fn begin_instance(&mut self, account: &str, id: u64) -> io::Result<u16> {
        let c = self
            .characters
            .get(&id)
            .filter(|c| c.account == account && !c.deleted)
            .ok_or_else(|| invalid("active character not owned"))?;
        let instance = c
            .instance
            .checked_add(1)
            .ok_or_else(|| invalid("character instance stamp exhausted"))?;
        self.append_record(&format!("e1\t{id}\t{instance}"))?;
        self.characters.get_mut(&id).unwrap().instance = instance;
        Ok(instance)
    }
    fn append_record(&mut self, line: &str) -> io::Result<()> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("journal revision exhausted"))?;
        if self.poisoned {
            return Err(invalid(
                "journal write failed previously; restart and inspect journal",
            ));
        }
        let record = format!("{line}\t{:08x}\n", hash(line.as_bytes()));
        if self
            .file
            .metadata()?
            .len()
            .saturating_add(record.len() as u64)
            > self.compact_after
        {
            self.compact_journal()?;
        }
        if self
            .file
            .metadata()?
            .len()
            .saturating_add(record.len() as u64)
            > MAX_JOURNAL_BYTES
        {
            return Err(invalid("journal exceeds 16 MiB limit"));
        }
        if let Err(e) = self
            .file
            .write_all(record.as_bytes())
            .and_then(|()| self.file.sync_all())
        {
            self.poisoned = true;
            return Err(e);
        }
        self.revision = revision;
        Ok(())
    }
    /// Bound repetitive progression history while retaining every non-progression
    /// transaction and verifying complete replay before replacing the file.
    pub fn compact_journal(&mut self) -> io::Result<bool> {
        if self.poisoned {
            return Err(invalid("cannot compact a poisoned journal"));
        }
        let mut raw = Vec::new();
        File::open(&self.journal_path)?
            .take(MAX_JOURNAL_BYTES + 1)
            .read_to_end(&mut raw)?;
        let mut lines = Vec::new();
        let before = match decode_journal_collect(&raw, Some(&mut lines)) {
            Ok(state) => state,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        if before.characters != self.characters || before.revision != self.revision {
            self.poisoned = true;
            return Err(invalid("journal differs from active character state"));
        }
        let base = self
            .revision
            .checked_sub(lines.len() as u64)
            .ok_or_else(|| invalid("compaction revision underflow"))?;
        let header = format!("j1\t{base}");
        let mut bytes = format!("{header}\t{:08x}\n", hash(header.as_bytes())).into_bytes();
        for line in lines {
            bytes.extend(line.as_bytes());
            bytes.push(b'\n');
        }
        let next_attempt = (raw.len() as u64)
            .saturating_add(1024 * 1024)
            .clamp(MAX_JOURNAL_BYTES / 2, MAX_JOURNAL_BYTES);
        if bytes.len() >= raw.len() {
            self.compact_after = next_attempt;
            return Ok(false);
        }
        let after = match decode_journal_state(&bytes) {
            Ok(state) => state,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        if after.characters != before.characters
            || after.revision != before.revision
            || after.relocations.len() != before.relocations.len()
            || before.relocations.iter().any(|(id, (_, position))| {
                after
                    .relocations
                    .get(id)
                    .is_none_or(|(_, other)| position != other)
            })
        {
            self.poisoned = true;
            return Err(invalid("compaction replay differs from original state"));
        }
        // Anchor all old relocation revisions before rebasing their replay order.
        // Both the old and compacted journals end at this same revision.
        self.positions_dirty = true;
        self.flush_positions()?;
        let temporary = self.journal_path.with_extension("journal.compact.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        let backup = self.journal_path.with_extension("journal.previous");
        fs::copy(&self.journal_path, &backup)?;
        OpenOptions::new().write(true).open(&backup)?.sync_all()?;
        fs::rename(&temporary, &self.journal_path)?;
        match OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.journal_path)
        {
            Ok(file) => self.file = file,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        }
        self.compact_after = (bytes.len() as u64)
            .saturating_add(1024 * 1024)
            .clamp(MAX_JOURNAL_BYTES / 2, MAX_JOURNAL_BYTES);
        eprintln!(
            "Compacted character journal: {} -> {} bytes at revision {}.",
            raw.len(),
            bytes.len(),
            self.revision
        );
        Ok(true)
    }
}

#[cfg(test)]
fn decode_positions(
    bytes: &[u8],
    characters: &BTreeMap<u64, Character>,
) -> io::Result<BTreeMap<u64, Placement>> {
    decode_position_snapshot(bytes, characters).map(|(_, positions)| positions)
}
fn decode_position_snapshot(
    bytes: &[u8],
    characters: &BTreeMap<u64, Character>,
) -> io::Result<(u64, BTreeMap<u64, Placement>)> {
    if bytes.len() < 16 || bytes.len() > 1024 * 1024 {
        return Err(invalid("invalid placement snapshot header/size"));
    }
    let (revision, start, base) = match &bytes[..8] {
        b"AC2POS01" => (0, 12, 16),
        b"AC2POS02" if bytes.len() >= 24 => {
            (u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 20, 24)
        }
        _ => return Err(invalid("invalid placement snapshot version")),
    };
    let count = u32::from_le_bytes(bytes[start - 4..start].try_into().unwrap()) as usize;
    if count.checked_mul(25).and_then(|n| n.checked_add(base)) != Some(bytes.len()) {
        return Err(invalid("invalid placement snapshot length"));
    }
    let end = bytes.len() - 4;
    if hash(&bytes[..end]) != u32::from_le_bytes(bytes[end..].try_into().unwrap()) {
        return Err(invalid("placement snapshot checksum mismatch"));
    }
    let mut positions = BTreeMap::new();
    for record in bytes[start..end].chunks_exact(25) {
        let id = u64::from_le_bytes(record[..8].try_into().unwrap());
        let placement = Placement {
            cell: u32::from_le_bytes(record[8..12].try_into().unwrap()),
            coordinates: std::array::from_fn(|i| {
                f32::from_le_bytes(record[12 + i * 4..16 + i * 4].try_into().unwrap())
            }),
            heading: record[24],
        };
        if !characters.contains_key(&id)
            || !placement.valid()
            || positions.insert(id, placement).is_some()
        {
            return Err(invalid("unknown, duplicate or invalid saved placement"));
        }
    }
    Ok((revision, positions))
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock);
    }
}

pub fn decode_journal(bytes: &[u8]) -> io::Result<BTreeMap<u64, Character>> {
    decode_journal_state(bytes).map(|journal| journal.characters)
}
struct JournalState {
    characters: BTreeMap<u64, Character>,
    relocations: BTreeMap<u64, (u64, Placement)>,
    revision: u64,
}
fn encode_placement(placement: Placement) -> String {
    format!(
        "{:08x},{:08x},{:08x},{:08x},{}",
        placement.cell,
        placement.coordinates[0].to_bits(),
        placement.coordinates[1].to_bits(),
        placement.coordinates[2].to_bits(),
        placement.heading
    )
}
fn decode_placement(text: &str) -> io::Result<Placement> {
    if text.len() > 64 {
        return Err(invalid("placement field limit"));
    }
    let fields: Vec<_> = text.split(',').collect();
    if fields.len() != 5 {
        return Err(invalid("placement field count"));
    }
    let word = |index: usize| {
        u32::from_str_radix(fields[index], 16).map_err(|_| invalid("invalid placement word"))
    };
    let placement = Placement {
        cell: word(0)?,
        coordinates: [
            f32::from_bits(word(1)?),
            f32::from_bits(word(2)?),
            f32::from_bits(word(3)?),
        ],
        heading: fields[4]
            .parse()
            .map_err(|_| invalid("invalid placement heading"))?,
    };
    if !placement.valid() {
        return Err(invalid("invalid persisted placement"));
    }
    Ok(placement)
}
fn decode_journal_state(bytes: &[u8]) -> io::Result<JournalState> {
    decode_journal_collect(bytes, None)
}
fn decode_journal_collect(
    bytes: &[u8],
    mut compacted: Option<&mut Vec<String>>,
) -> io::Result<JournalState> {
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(invalid("journal exceeds 16 MiB limit"));
    }
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        return Err(invalid("incomplete character journal tail"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("invalid character journal encoding"))?;
    let mut records: BTreeMap<u64, Character> = BTreeMap::new();
    let mut relocations = BTreeMap::new();
    let mut revision = 0u64;
    let mut run = compaction::Run::default();
    for (index, line) in text.lines().enumerate() {
        revision = revision
            .checked_add(1)
            .ok_or_else(|| invalid("journal revision exhausted"))?;
        let (data, sum) = line
            .rsplit_once('\t')
            .ok_or_else(|| invalid("missing journal checksum"))?;
        if u32::from_str_radix(sum, 16).ok() != Some(hash(data.as_bytes())) {
            return Err(invalid("character journal checksum mismatch"));
        }
        let fields: Vec<_> = data.split('\t').collect();
        if fields.len() == 2 && fields[0] == "j1" {
            if index != 0 {
                return Err(invalid("journal revision base must be first"));
            }
            revision = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid journal revision base"))?;
            continue;
        }
        if let Some(output) = compacted.as_mut() {
            if !(fields.len() == 3 && matches!(fields[0], "g1" | "g2" | "g3")) {
                run.flush(output);
                output.push(line.to_owned());
            }
        }
        if fields.len() == 3 && matches!(fields[0], "q1" | "q3" | "q4" | "q5" | "q6") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid quest owner"))?;
            let command = crate::quests::Command::decode(fields[2]).map_err(io::Error::other)?;
            if !(fields[0] == "q1" && matches!(command, crate::quests::Command::Accept | crate::quests::Command::Claim)
                || fields[0] == "q3" && matches!(command, crate::quests::Command::AcceptSupplies)
                || fields[0] == "q4" && matches!(command, crate::quests::Command::AcceptPotion | crate::quests::Command::ClaimPotion | crate::quests::Command::AcceptJourney | crate::quests::Command::ClaimJourney)
                || fields[0] == "q5" && matches!(command, crate::quests::Command::AcceptSkill | crate::quests::Command::UsedSkill | crate::quests::Command::ClaimSkill)
                || fields[0] == "q6" && matches!(command, crate::quests::Command::AcceptHunt | crate::quests::Command::ClaimHunt)) {
                return Err(invalid("quest command does not match its journal revision"));
            }
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted && c.progression.alive())
                .ok_or_else(|| invalid("unavailable quest owner"))?;
            character.quests = character.quests.apply(&command).map_err(io::Error::other)?;
            for reward in crate::quests::rewards(&command) {
                character.progression = character
                    .progression
                    .apply(&reward)
                    .map_err(io::Error::other)?;
            }
            continue;
        }
        if fields.len() == 4 && fields[0] == "q2" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid quest reward owner"))?;
            let command = crate::quests::Command::decode(&format!("k,{}", fields[2]))
                .map_err(io::Error::other)?;
            let changes = crate::player_transaction::decode_changes(fields[3])?;
            if changes.is_empty() {
                return Err(invalid("quest kill payout is empty"));
            }
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted && c.progression.alive())
                .ok_or_else(|| invalid("unavailable quest kill owner"))?;
            let quests = character.quests.apply(&command).map_err(io::Error::other)?;
            if quests == character.quests {
                return Err(invalid("quest kill did not advance an objective"));
            }
            for change in changes {
                character.progression = character
                    .progression
                    .apply(&change)
                    .map_err(io::Error::other)?;
            }
            character.quests = quests;
            continue;
        }
        if fields.len() == 3 && fields[0] == "g3" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid progression checkpoint character"))?;
            let character = records
                .get_mut(&id)
                .filter(|character| !character.deleted)
                .ok_or_else(|| invalid("unavailable progression checkpoint character"))?;
            character.progression = compaction::progression(fields[2], &character.progression)?;
            if compacted.is_some() {
                run.record(id, &character.progression, line);
            }
            continue;
        }
        if fields.len() == 2 && fields[0] == "k4" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid training attack character"))?;
            let character = records
                .get_mut(&id)
                .filter(|c| {
                    !c.deleted
                        && c.training_kit
                        && c.skills.get(crate::skills::TRAINING_ATTACK).is_none()
                })
                .ok_or_else(|| invalid("unavailable training attack character"))?;
            character.skills = character
                .skills
                .with_training_attack(
                    &character.progression,
                    character
                        .creation
                        .as_ref()
                        .map_or(1, |request| request.race),
                )
                .map_err(|_| invalid("invalid training attack grant"))?;
            continue;
        }
        if fields.len() == 2 && fields[0] == "k5" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid starter skill character"))?;
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("unavailable starter skill character"))?;
            let request = character
                .creation
                .as_ref()
                .ok_or_else(|| invalid("starter skills without a creation profile"))?;
            character.skills = character
                .skills
                .with_starter_skills(&character.progression, request.race, request.sex)
                .map_err(|_| invalid("invalid starter skill grant"))?;
            continue;
        }
        if fields.len() == 2 && fields[0] == "k3" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid recall grant character"))?;
            let character = records
                .get_mut(&id)
                .filter(|character| {
                    !character.deleted
                        && character.binding.is_some()
                        && character
                            .skills
                            .get(crate::skills::LIFESTONE_RECALL)
                            .is_none()
                })
                .ok_or_else(|| invalid("unavailable recall grant character"))?;
            character.skills = character
                .skills
                .with_lifestone_recall(
                    &character.progression,
                    character
                        .creation
                        .as_ref()
                        .map_or(1, |request| request.race),
                )
                .map_err(|_| invalid("invalid recall grant"))?;
            continue;
        }
        if fields.len() == 4 && fields[0] == "k2" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid advancement character"))?;
            let skill = u32::from_str_radix(fields[2], 16)
                .map_err(|_| invalid("invalid advancement skill"))?;
            let paid = fields[3]
                .parse::<u64>()
                .map_err(|_| invalid("invalid advancement payment"))?;
            let character = records
                .get_mut(&id)
                .filter(|character| !character.deleted)
                .ok_or_else(|| invalid("unavailable advancement character"))?;
            let race = character
                .creation
                .as_ref()
                .map_or(1, |request| request.race);
            let (skills, progression) = character
                .skills
                .advance(skill, &character.progression, race)
                .map_err(|_| invalid("invalid skill advancement transition"))?;
            if character
                .progression
                .available_xp
                .checked_sub(progression.available_xp)
                != Some(paid)
                || paid == 0
            {
                return Err(invalid("advancement payment differs from immutable curve"));
            }
            character.skills = skills;
            character.progression = progression;
            continue;
        }
        if fields.len() == 4 && fields[0] == "p3" {
            let id = fields[1].parse::<u64>().map_err(|_| invalid("Invalid travel lesson owner"))?;
            let portal = u64::from_str_radix(fields[2], 16).map_err(|_| invalid("Invalid training portal identity"))?;
            if !crate::quests::is_training_portal(portal) { return Err(invalid("Travel lesson portal is not enabled")); }
            let placement = decode_placement(fields[3])?;
            let character = records.get_mut(&id).filter(|c| !c.deleted && c.progression.alive()).ok_or_else(|| invalid("Unavailable travel lesson owner"))?;
            character.quests = character.quests.apply(&crate::quests::Command::Travelled).map_err(io::Error::other)?;
            relocations.insert(id, (revision, placement));
            continue;
        }
        if fields.len() == 3 && matches!(fields[0], "l1" | "l2" | "l3" | "p1" | "p2" | "r2") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid lifestone character"))?;
            let placement = decode_placement(fields[2])?;
            let character = records
                .get_mut(&id)
                .filter(|character| !character.deleted)
                .ok_or_else(|| invalid("unavailable lifestone character"))?;
            if matches!(fields[0], "l1" | "l2" | "l3") {
                if !character.progression.alive() {
                    return Err(invalid("dead character binding"));
                }
                if fields[0] == "l3" { character.quests = character.quests.apply(&crate::quests::Command::BoundLifestone).map_err(io::Error::other)?; }
                character.binding = Some(placement);
                if matches!(fields[0], "l2" | "l3") {
                    character.skills = character
                        .skills
                        .with_lifestone_recall(
                            &character.progression,
                            character
                                .creation
                                .as_ref()
                                .map_or(1, |request| request.race),
                        )
                        .map_err(|_| invalid("invalid bound recall skill"))?;
                }
            } else {
                if fields[0] != "p2" && character.binding != Some(placement) {
                    return Err(invalid("relocation differs from saved binding"));
                }
                if fields[0] == "r2" {
                    character.progression = character
                        .progression
                        .apply(&crate::progression::Command::Resurrect)
                        .map_err(|_| invalid("invalid bound resurrection"))?;
                } else if !character.progression.alive() {
                    return Err(invalid("dead character recall"));
                }
                relocations.insert(id, (revision, placement));
            }
            continue;
        }
        if fields.len() == 3 && fields[0] == "t4" {
            let id = fields[1].parse::<u64>().map_err(|_| invalid("invalid starter owner"))?;
            let first = fields[2].parse::<u64>().map_err(|_| invalid("invalid starter identity"))?;
            let high = records.values().map(|character| character.inventory.high_water()).max()
                .unwrap_or(crate::inventory::ITEM_NAMESPACE);
            if high.checked_add(1) != Some(first) { return Err(invalid("nonsequential starter identity")); }
            let character = records.get_mut(&id)
                .filter(|character| !character.deleted && !character.starter_gear)
                .ok_or_else(|| invalid("invalid repeated starter grant"))?;
            let request = character.creation.as_ref().ok_or_else(|| invalid("starter grant without creation"))?;
            character.inventory = crate::starter_gear::candidate(request, &character.inventory, first)?;
            character.starter_gear = true;
            character.training_kit = true;
            continue;
        }
        if fields.len() == 3 && fields[0] == "t3" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("training supplies owner"))?;
            let first = fields[2]
                .parse::<u64>()
                .map_err(|_| invalid("training supplies identity"))?;
            let high = records
                .values()
                .map(|c| c.inventory.high_water())
                .max()
                .unwrap_or(crate::inventory::ITEM_NAMESPACE);
            if high.checked_add(1) != Some(first) {
                return Err(invalid("nonsequential training supplies identity"));
            }
            let character = records
                .get_mut(&id)
                .filter(|c| {
                    !c.deleted && c.training_kit && !c.training_supplies && c.progression.alive()
                })
                .ok_or_else(|| invalid("invalid repeated training supplies grant"))?;
            character.inventory =
                crate::training_kit::supply_inventory(&character.inventory, first)
                    .map_err(io::Error::other)?;
            character.training_supplies = true;
            continue;
        }
        if fields.len() == 3 && matches!(fields[0], "t1" | "t2") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid training kit owner"))?;
            let item = fields[2]
                .parse::<u64>()
                .map_err(|_| invalid("invalid training kit item"))?;
            let high = records
                .values()
                .map(|character| character.inventory.high_water())
                .max()
                .unwrap_or(crate::inventory::ITEM_NAMESPACE);
            if high.checked_add(1) != Some(item) {
                return Err(invalid("nonsequential training kit identity"));
            }
            let character = records
                .get_mut(&id)
                .filter(|character| !character.deleted && !character.training_kit)
                .ok_or_else(|| invalid("invalid repeated training kit grant"))?;
            if !character.creation.as_ref().is_some_and(|request| {
                crate::appearance::body(request.race, request.sex).is_some()
                    && (fields[0] == "t1" || request.race == crate::appearance::HUMAN)
            }) {
                return Err(invalid("unsupported training kit character profile"));
            }
            let slot = character
                .inventory
                .free_slot()
                .ok_or_else(|| invalid("training kit bag full"))?;
            character.inventory = character
                .inventory
                .apply(&crate::inventory::Command::Grant(
                    crate::training_kit::item(item, slot),
                ))
                .map_err(|_| invalid("invalid training kit inventory transition"))?;
            if fields[0] == "t2" {
                character.skills = character
                    .skills
                    .with_training_attack(
                        &character.progression,
                        character.creation.as_ref().unwrap().race,
                    )
                    .map_err(|_| invalid("invalid training kit skill transition"))?;
            }
            character.training_kit = true;
            continue;
        }
        if fields.len() == 3 && fields[0] == "k1" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid skill owner"))?;
            let command = crate::skills::Command::decode(fields[2])
                .map_err(|_| invalid("invalid skill mutation"))?;
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("skill owner unavailable"))?;
            let race = character
                .creation
                .as_ref()
                .map_or(1, |request| request.race);
            character.skills = character
                .skills
                .apply(command, &character.progression, race)
                .map_err(|_| invalid("invalid saved skill transition"))?;
            continue;
        }
        if fields.len() == 3 && fields[0] == "g2" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid progression batch owner"))?;
            if fields[2].len() > 256 {
                return Err(invalid("progression batch limit"));
            }
            let commands = fields[2].split(';').collect::<Vec<_>>();
            if commands.is_empty() || commands.len() > 8 {
                return Err(invalid("progression batch size"));
            }
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("progression batch owner unavailable"))?;
            let mut next = character.progression.clone();
            for text in commands {
                let command = crate::progression::Command::decode(text)
                    .map_err(|_| invalid("invalid progression batch event"))?;
                next = next
                    .apply(&command)
                    .map_err(|_| invalid("invalid progression batch transition"))?;
            }
            character.progression = next;
            if compacted.is_some() {
                run.record(id, &character.progression, line);
            }
            continue;
        }
        if fields.len() == 3 && fields[0] == "g1" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid progression character ID"))?;
            let command = crate::progression::Command::decode(fields[2])
                .map_err(|_| invalid("invalid progression event"))?;
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("progression event references unavailable character"))?;
            character.progression = character
                .progression
                .apply(&command)
                .map_err(|_| invalid("invalid progression transition"))?;
            if compacted.is_some() {
                run.record(id, &character.progression, line);
            }
            continue;
        }
        if fields.len() == 5 && fields[0] == "a2" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid encounter transaction owner"))?;
            if fields[2].len() != 8 {
                return Err(invalid("invalid encounter entity encoding"));
            }
            let entity = u32::from_str_radix(fields[2], 16)
                .map_err(|_| invalid("invalid encounter entity"))?;
            if entity >> 24 != 0x47 {
                return Err(invalid("invalid encounter entity type"));
            }
            let items = crate::player_transaction::decode_items(fields[3])?;
            let changes = crate::player_transaction::decode_changes(fields[4])?;
            if items.is_empty() || changes.is_empty() {
                return Err(invalid("empty encounter item payout"));
            }
            let first = if items.iter().any(|command| command.allocated_id().is_some()) {
                Some(
                    records
                        .values()
                        .map(|c| c.inventory.high_water())
                        .max()
                        .unwrap_or(crate::inventory::ITEM_NAMESPACE)
                        .checked_add(1)
                        .ok_or_else(|| invalid("encounter item identity exhausted"))?,
                )
            } else {
                None
            };
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted && c.progression.alive())
                .ok_or_else(|| invalid("unavailable encounter payout owner"))?;
            let (inventory, progression) = crate::player_transaction::candidate(
                &character.inventory,
                &character.progression,
                &items,
                &changes,
                first,
            )?;
            character.quests = character.quests.record_kill(entity);
            character.inventory = inventory;
            character.progression = progression;
            continue;
        }
        if fields.len() == 4 && matches!(fields[0], "a1" | "a2" | "a3") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid transaction owner"))?;
            let items = crate::player_transaction::decode_items(fields[2])?;
            let changes = crate::player_transaction::decode_changes(fields[3])?;
            let first = if items.iter().any(|command| command.allocated_id().is_some()) {
                Some(
                    records
                        .values()
                        .map(|c| c.inventory.high_water())
                        .max()
                        .unwrap_or(crate::inventory::ITEM_NAMESPACE)
                        .checked_add(1)
                        .ok_or_else(|| invalid("transaction item identity exhausted"))?,
                )
            } else {
                None
            };
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("unavailable transaction owner"))?;
            let (inventory, progression) = crate::player_transaction::candidate(
                &character.inventory,
                &character.progression,
                &items,
                &changes,
                first,
            )?;
            if fields[0] == "a2" {
                if !character.progression.alive() { return Err(invalid("Dead supply exchange owner")); }
                character.quests = crate::quests::validate_supplies_exchange(&character.quests, &character.inventory, &items, &changes, first.ok_or_else(|| invalid("Missing supply output identity"))?)?;
            }
            if fields[0] == "a3" {
                if !character.progression.alive() { return Err(invalid("Dead practice craft owner")); }
                character.quests = crate::quests::validate_practice_craft(&character.quests, &character.inventory, &items, &changes, first.ok_or_else(|| invalid("Missing craft output identity"))?)?;
            }
            character.inventory = inventory;
            character.progression = progression;
            continue;
        }
        if fields.len() == 3 && fields[0] == "i1" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid inventory character"))?;
            let command = crate::inventory::Command::decode(fields[2])
                .map_err(|_| invalid("invalid inventory event"))?;
            if let Some(allocated) = command.allocated_id() {
                let high = records
                    .values()
                    .map(|c| c.inventory.high_water())
                    .max()
                    .unwrap_or(crate::inventory::ITEM_NAMESPACE);
                if high.checked_add(1) != Some(allocated) {
                    return Err(invalid("invalid global item identity progression"));
                }
            }
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("inventory event references unavailable character"))?;
            character.inventory = character
                .inventory
                .apply(&command)
                .map_err(|_| invalid("invalid inventory transition"))?;
            continue;
        }
        if fields.len() == 3 && matches!(fields[0], "u1" | "h1" | "o1") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid UI character ID"))?;
            let character = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("UI event references unavailable character"))?;
            let hex = fields[2].as_bytes();
            if hex.len() > crate::preferences::MAX_BYTES * 2 || hex.len() % 2 != 0 {
                return Err(invalid("invalid UI settings encoding size"));
            }
            let mut bytes = Vec::with_capacity(hex.len() / 2);
            for pair in hex.chunks_exact(2) {
                let high = (pair[0] as char)
                    .to_digit(16)
                    .ok_or_else(|| invalid("invalid UI hex"))?;
                let low = (pair[1] as char)
                    .to_digit(16)
                    .ok_or_else(|| invalid("invalid UI hex"))?;
                bytes.push(((high << 4) | low) as u8);
            }
            if fields[0] == "u1" {
                character.ui = Some(
                    crate::preferences::UiSettings::decode(&bytes)
                        .map_err(|_| invalid("invalid saved UI settings"))?,
                );
            } else if fields[0] == "o1" {
                character.gameplay_options = Some(
                    crate::preferences::GameplayOptions::decode(&bytes)
                        .map_err(|_| invalid("invalid saved gameplay options"))?,
                );
            } else {
                let update = crate::preferences::HotbarUpdate::decode(&bytes)
                    .map_err(|_| invalid("invalid saved hotbar update"))?;
                character.hotbar = character
                    .hotbar
                    .apply(&update)
                    .map_err(|_| invalid("saved hotbar exceeds limit"))?;
            }
            continue;
        }
        if fields.len() == 3 && fields[0] == "e1" {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid instance character ID"))?;
            let instance = fields[2]
                .parse::<u16>()
                .map_err(|_| invalid("invalid instance stamp"))?;
            let c = records
                .get_mut(&id)
                .filter(|c| !c.deleted)
                .ok_or_else(|| invalid("instance event references unavailable character"))?;
            if c.instance.checked_add(1) != Some(instance) {
                return Err(invalid("invalid instance stamp progression"));
            }
            c.instance = instance;
            continue;
        }
        if fields.len() == 2 && matches!(fields[0], "d1" | "r1") {
            let id = fields[1]
                .parse::<u64>()
                .map_err(|_| invalid("invalid character lifecycle ID"))?;
            let c = records
                .get(&id)
                .ok_or_else(|| invalid("lifecycle event references unknown character"))?;
            let deleting = fields[0] == "d1";
            if c.deleted == deleting {
                return Err(invalid("invalid character lifecycle transition"));
            }
            if !deleting
                && records
                    .values()
                    .filter(|other| other.account == c.account && !other.deleted)
                    .count()
                    >= 5
            {
                return Err(invalid("restoration exceeds local character limit"));
            }
            records.get_mut(&id).unwrap().deleted = deleting;
            continue;
        }
        if !((fields.len() == 4 && fields[0] == "v1") || (fields.len() == 5 && fields[0] == "v2"))
            || !valid_name(fields[2])
            || !valid_name(fields[3])
        {
            return Err(invalid("invalid journal record"));
        }
        let id = fields[1]
            .parse::<u64>()
            .map_err(|_| invalid("invalid character ID"))?;
        if id == 0 || records.contains_key(&id) {
            return Err(invalid("duplicate or zero character ID"));
        }
        if records
            .values()
            .any(|c: &Character| c.name.eq_ignore_ascii_case(fields[3]))
        {
            return Err(invalid("duplicate character name in journal"));
        }
        if records
            .values()
            .filter(|c| c.account == fields[2] && !c.deleted)
            .count()
            >= 5
        {
            return Err(invalid("account exceeds local character limit"));
        }
        let creation = if fields[0] == "v2" {
            Some(decode_creation(fields[3], fields[4])?)
        } else {
            None
        };
        records.insert(
            id,
            Character {
                id,
                account: fields[2].into(),
                name: fields[3].into(),
                creation,
                deleted: false,
                instance: 0,
                ui: None,
                gameplay_options: None,
                hotbar: crate::preferences::Hotbar::default(),
                inventory: crate::inventory::Inventory::default(),
                progression: crate::progression::Progression::default(),
                skills: crate::skills::Book::default(),
                quests: crate::quests::Book::default(),
                training_kit: false,
                training_supplies: false,
                starter_gear: false,
                binding: None,
            },
        );
    }
    if let Some(output) = compacted {
        run.flush(output);
    }
    Ok(JournalState {
        characters: records,
        relocations,
        revision,
    })
}

// Journal v2 uses canonical little-endian words encoded as hex, independent of network
// string padding. Physical tuple order and float bits survive without normalization.
fn encode_creation(request: &CharacterCreationRequest) -> io::Result<String> {
    if request.physical_values.len() > MAX_PHYSICAL_VALUES {
        return Err(invalid("too many physical values"));
    }
    let mut keys = std::collections::BTreeSet::new();
    if request
        .physical_values
        .iter()
        .any(|(key, bits)| !keys.insert(*key) || !f32::from_bits(*bits).is_finite())
    {
        return Err(invalid("invalid or duplicate physical value"));
    }
    let mut bytes = Vec::with_capacity(28 + request.physical_values.len() * 8);
    for word in [
        request.entity_did,
        request.start_area,
        request.race,
        request.sex,
        request.class,
        request.implement,
        request.physical_values.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for (key, bits) in &request.physical_values {
        bytes.extend(key.to_le_bytes());
        bytes.extend(bits.to_le_bytes());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    Ok(out)
}
fn decode_creation(name: &str, hex: &str) -> io::Result<CharacterCreationRequest> {
    if hex.len() < 56 || hex.len() > (28 + MAX_PHYSICAL_VALUES * 8) * 2 || hex.len() % 8 != 0 {
        return Err(invalid("invalid native character data length"));
    }
    fn nibble(b: u8) -> io::Result<u8> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(invalid("invalid native character hex")),
        }
    }
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|p| Ok((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect::<io::Result<_>>()?;
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
        .collect();
    let count = words[6] as usize;
    if count > MAX_PHYSICAL_VALUES || words.len() != 7 + count * 2 {
        return Err(invalid("invalid native character physical count"));
    }
    let request = CharacterCreationRequest {
        name: name.into(),
        entity_did: words[0],
        start_area: words[1],
        race: words[2],
        sex: words[3],
        class: words[4],
        implement: words[5],
        physical_values: words[7..].chunks_exact(2).map(|w| (w[0], w[1])).collect(),
    };
    encode_creation(&request)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_restoration_keeps_identity_appearance_and_saved_placement() {
        let dir = std::env::temp_dir().join(format!("ac2-lifecycle-{}", std::process::id()));
        {
            let mut store = Store::open(&dir).unwrap();
            for i in 1..=5 {
                store
                    .create_native("local", &request(&format!("Hero {i}")))
                    .unwrap();
            }
            store.set_placement(5, Placement::HUMAN_START).unwrap();
            store.flush_positions().unwrap();
            assert!(store.delete("other", 5).is_err());
            store.delete("local", 5).unwrap();
            assert!(store.set_placement(5, Placement::HUMAN_START).is_err());
            assert!(store.delete("local", 5).is_err());
            assert_eq!(store.create("local", "Sixth Hero").unwrap().id, 6);
            assert!(store.restore("local", 5).is_err());
            store.delete("local", 6).unwrap();
            store.restore("local", 5).unwrap();
            assert!(store.restore("other", 6).is_err());
            assert!(store.create("other", "Sixth Hero").is_err());
        }
        {
            let store = Store::open(&dir).unwrap();
            assert!(!store.characters[&5].deleted);
            assert!(store.characters[&6].deleted);
            assert_eq!(store.characters[&5].creation, Some(request("Hero 5")));
            assert_eq!(store.placement(5), Some(Placement::HUMAN_START));
        }
        for line in ["d1\t1", "r1\t1"] {
            assert!(decode_journal(&record(line)).is_err());
        }
        let mut bad = record("v1\t1\tlocal\tHero");
        bad.extend(record("r1\t1"));
        assert!(decode_journal(&bad).is_err());
        for file in ["characters.journal", "positions.snapshot"] {
            fs::remove_file(dir.join(file)).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn placement_snapshots_replace_durably_and_reject_corruption() {
        let dir = std::env::temp_dir().join(format!("ac2-positions-{}", std::process::id()));
        let mut final_position = Placement::HUMAN_START;
        {
            let mut store = Store::open(&dir).unwrap();
            store.create("local", "Explorer").unwrap();
            assert!(store.set_placement(2, final_position).is_err());
            store.set_placement(1, final_position).unwrap();
            store.flush_positions().unwrap();
            final_position.coordinates[0] += 4.0;
            store.set_placement(1, final_position).unwrap();
            store.flush_positions().unwrap();
        }
        {
            let store = Store::open(&dir).unwrap();
            assert_eq!(store.placement(1), Some(final_position));
            let bytes = fs::read(dir.join("positions.snapshot")).unwrap();
            for end in 0..bytes.len() {
                assert!(decode_positions(&bytes[..end], &store.characters).is_err());
            }
            let mut corrupt = bytes;
            corrupt[20] ^= 1;
            assert!(decode_positions(&corrupt, &store.characters).is_err());
            // A valid checksum is insufficient when a placement references no character.
            assert!(
                decode_positions(
                    &fs::read(dir.join("positions.snapshot")).unwrap(),
                    &BTreeMap::new()
                )
                .is_err()
            );
        }
        fs::remove_file(dir.join("characters.journal")).unwrap();
        fs::remove_file(dir.join("positions.snapshot")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    fn request(name: &str) -> CharacterCreationRequest {
        CharacterCreationRequest {
            name: name.into(),
            entity_did: 0x10001234,
            start_area: 7,
            race: 2,
            sex: 1,
            class: 3,
            implement: 4,
            physical_values: vec![(9, (-0.0f32).to_bits()), (2, 0.1234567f32.to_bits())],
        }
    }
    fn record(data: &str) -> Vec<u8> {
        format!("{data}\t{:08x}\n", hash(data.as_bytes())).into_bytes()
    }
    #[test]
    fn avatar_lifetimes_survive_restart_and_reject_invalid_journal_transitions() {
        let dir = std::env::temp_dir().join(format!("ac2-instances-{}", std::process::id()));
        {
            let mut store = Store::open(&dir).unwrap();
            store.create("local", "Hero").unwrap();
            assert!(store.begin_instance("other", 1).is_err());
            assert_eq!(store.begin_instance("local", 1).unwrap(), 1);
            store.delete("local", 1).unwrap();
            assert!(store.begin_instance("local", 1).is_err());
            store.restore("local", 1).unwrap();
        }
        {
            let mut store = Store::open(&dir).unwrap();
            assert_eq!(store.begin_instance("local", 1).unwrap(), 2);
            let length = store.file.metadata().unwrap().len();
            store.characters.get_mut(&1).unwrap().instance = u16::MAX;
            assert!(store.begin_instance("local", 1).is_err());
            assert_eq!(store.file.metadata().unwrap().len(), length);
        }
        let prefix = record("v1\t1\tlocal\tHero");
        for suffix in ["e1\t1\t0", "e1\t1\t2", "e1\t2\t1", "e1\t1\t65536"] {
            let mut bytes = prefix.clone();
            bytes.extend(record(suffix));
            assert!(decode_journal(&bytes).is_err());
        }
        let mut bytes = prefix;
        bytes.extend(record("e1\t1\t1"));
        let valid = bytes.clone();
        bytes.extend(record("e1\t1\t1"));
        assert!(decode_journal(&bytes).is_err());
        bytes = valid;
        bytes.extend(record("d1\t1"));
        bytes.extend(record("e1\t1\t2"));
        assert!(decode_journal(&bytes).is_err());
        fs::remove_file(dir.join("characters.journal")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn mixed_versions_retain_exact_native_appearance_after_restart() {
        let dir = std::env::temp_dir().join(
            format!(
                "ac2-native-journal-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )
            .replace(['(', ')'], ""),
        );
        let original = request("Native Hero");
        {
            let mut store = Store::open(&dir).unwrap();
            assert_eq!(store.create("local", "Legacy Hero").unwrap().creation, None);
            let created = store.create_native("local", &original).unwrap();
            assert_eq!(created.creation.as_ref(), Some(&original));
            assert!(
                store
                    .create_native("other", &request("native hero"))
                    .is_err()
            );
            for i in 0..3 {
                store
                    .create_native("local", &request(&format!("Extra {i}")))
                    .unwrap();
            }
            assert!(store.create_native("local", &request("Sixth")).is_err());
        }
        {
            let store = Store::open(&dir).unwrap();
            assert_eq!(store.characters.len(), 5);
            assert_eq!(store.characters[&1].creation, None);
            assert_eq!(store.characters[&2].creation.as_ref(), Some(&original));
        }
        fs::remove_file(dir.join("characters.journal")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn native_records_reject_torn_corrupt_and_semantically_malformed_data() {
        let req = request("Hero");
        let encoded = encode_creation(&req).unwrap();
        let good = record(&format!("v2\t1\tlocal\tHero\t{encoded}"));
        assert_eq!(
            decode_journal(&good).unwrap()[&1].creation.as_ref(),
            Some(&req)
        );
        for n in 1..good.len() {
            assert!(
                decode_journal(&good[..n]).is_err(),
                "accepted torn prefix {n}"
            );
        }
        for n in 0..encoded.len() {
            assert!(
                decode_journal(&record(&format!("v2\t1\tlocal\tHero\t{}", &encoded[..n]))).is_err()
            );
        }
        let mut malformed = encoded.clone();
        malformed.replace_range(48..56, "ffffffff");
        assert!(decode_journal(&record(&format!("v2\t1\tlocal\tHero\t{malformed}"))).is_err());
        let mut invalid = req.clone();
        invalid.physical_values.push(invalid.physical_values[0]);
        assert!(encode_creation(&invalid).is_err());
        invalid.physical_values = vec![(1, f32::NAN.to_bits())];
        assert!(encode_creation(&invalid).is_err());
        invalid.physical_values = (0..257).map(|i| (i, 0)).collect();
        assert!(encode_creation(&invalid).is_err());
        let mut mixed = record("v1\t1\tlocal\tHero");
        mixed.extend(record("v1\t2\tother\thero"));
        assert!(decode_journal(&mixed).is_err());
        let mut too_many = Vec::new();
        for i in 1..=6 {
            too_many.extend(record(&format!("v1\t{i}\tlocal\tHero {i}")));
        }
        assert!(decode_journal(&too_many).is_err());
    }
    #[test]
    fn persistence_exclusive_writer_and_corruption() {
        let dir = std::env::temp_dir().join(
            format!(
                "ac2-journal-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )
            .replace(['(', ')'], ""),
        );
        fs::create_dir_all(&dir).unwrap();
        {
            let mut s = Store::open(&dir).unwrap();
            assert!(Store::open(&dir).is_err());
            assert_eq!(s.create("local", "Explorer").unwrap().id, 1);
            assert!(s.create("local", "explorer").is_err());
        }
        {
            let s = Store::open(&dir).unwrap();
            assert_eq!(s.characters[&1].name, "Explorer");
        }
        let path = dir.join("characters.journal");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] ^= 1;
        assert!(decode_journal(&bytes).is_err());
        assert!(decode_journal(&bytes[..bytes.len() - 1]).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
