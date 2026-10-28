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
// New authored profile, separate from the immutable original training reward.
pub const SUPPLIES_QUEST: u32 = 0x4000ff02;
pub const SUPPLIES_XP: u64 = 80;
pub const SUPPLIES_GOLD: u32 = 6;
pub const POTION_QUEST: u32 = 0x4000ff03;
pub const JOURNEY_QUEST: u32 = 0x4000ff04;
pub const SKILL_QUEST: u32 = 0x4000ff05;
pub const SKILL_XP: u64 = 150;
pub const SKILL_GOLD: u32 = 15;
/// Creatures defeated anywhere after the training lessons. Authored profile.
pub const HUNT_QUEST: u32 = 0x4000ff06;
pub const HUNT_GOAL: u8 = 8;
pub const HUNT_XP: u64 = 200;
pub const HUNT_GOLD: u32 = 25;
pub fn rewards(command: &Command) -> Vec<crate::progression::Command> {
    if matches!(command, Command::ClaimHunt) {
        vec![crate::progression::Command::AwardXp(HUNT_XP), crate::progression::Command::AwardGold(HUNT_GOLD)]
    } else if matches!(command, Command::ClaimSkill) {
        vec![crate::progression::Command::AwardXp(SKILL_XP), crate::progression::Command::AwardGold(SKILL_GOLD)]
    } else if matches!(command, Command::Claim) {
        vec![
            crate::progression::Command::AwardXp(REWARD_XP),
            crate::progression::Command::AwardGold(REWARD_GOLD),
        ]
    } else if matches!(command, Command::ClaimSupplies) {
        vec![crate::progression::Command::AwardXp(SUPPLIES_XP), crate::progression::Command::AwardGold(SUPPLIES_GOLD)]
    } else if matches!(command, Command::ClaimPotion | Command::ClaimJourney) {
        vec![crate::progression::Command::AwardXp(80), crate::progression::Command::AwardGold(if matches!(command, Command::ClaimPotion) { 8 } else { 10 })]
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
    supplies: Option<bool>,
    potion: u8, // 0 absent, 1 accepted, 2 crafted, 3 rewarded
    journey: u8, // 0 absent, 1 accepted, 2 bound, 3 travelled, 4 rewarded
    skill: u8, // 0 absent, 1 accepted, 2 used, 3 rewarded
    hunt: u8, // 0 absent, 1 accepted, 2 rewarded
    hunt_kills: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Accept,
    Kill(u32),
    Claim,
    AcceptSupplies,
    ClaimSupplies,
    AcceptPotion,
    CraftedPotion,
    ClaimPotion,
    AcceptJourney,
    BoundLifestone,
    Travelled,
    ClaimJourney,
    AcceptSkill,
    UsedSkill,
    ClaimSkill,
    AcceptHunt,
    ClaimHunt,
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
            Self::NotReady => "Finish the current training objective before claiming its reward.",
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
            Self::AcceptSupplies => "s.a".into(),
            Self::ClaimSupplies => "s.c".into(),
            Self::AcceptPotion => "p.a".into(), Self::CraftedPotion => "p.f".into(), Self::ClaimPotion => "p.c".into(),
            Self::AcceptJourney => "j.a".into(), Self::BoundLifestone => "j.b".into(), Self::Travelled => "j.t".into(), Self::ClaimJourney => "j.c".into(),
            Self::AcceptSkill => "u.a".into(), Self::UsedSkill => "u.f".into(), Self::ClaimSkill => "u.c".into(),
            Self::AcceptHunt => "h.a".into(), Self::ClaimHunt => "h.c".into(),
        }
    }
    pub fn decode(text: &str) -> Result<Self, QuestError> {
        match text {
            "a" => Ok(Self::Accept),
            "c" => Ok(Self::Claim),
            "s.a" => Ok(Self::AcceptSupplies),
            "s.c" => Ok(Self::ClaimSupplies),
            "p.a" => Ok(Self::AcceptPotion), "p.f" => Ok(Self::CraftedPotion), "p.c" => Ok(Self::ClaimPotion),
            "j.a" => Ok(Self::AcceptJourney), "j.b" => Ok(Self::BoundLifestone), "j.t" => Ok(Self::Travelled), "j.c" => Ok(Self::ClaimJourney),
            "u.a" => Ok(Self::AcceptSkill), "u.f" => Ok(Self::UsedSkill), "u.c" => Ok(Self::ClaimSkill),
            "h.a" => Ok(Self::AcceptHunt), "h.c" => Ok(Self::ClaimHunt),
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
    /// Shared player guidance for mentor speech, quest commands and the journal.
    /// The quest state and immutable rewards remain the persisted authored profile.
    pub fn guidance(&self) -> String {
        if self.hunt != 0 { return self.hunt_guidance(); }
        if self.skill != 0 { return self.skill_guidance(); }
        if self.journey != 0 { return self.journey_guidance(); }
        if self.potion != 0 { return self.potion_guidance(); }
        if self.supplies.is_some() { return self.supplies_guidance(); }
        match self.training {
            None => "Speak to a training mentor to begin Training Grounds, or use @quest start.".into(),
            Some(progress) if progress.claimed => format!(
                "Training complete. You received {REWARD_XP} experience and {REWARD_GOLD} gold. Speak to a training mentor again for the Field Supplies lesson."
            ),
            Some(progress) if progress.killed == GOAL => format!(
                "Training creatures defeated: {GOAL}/{GOAL}. Leave combat and speak to a training mentor for {REWARD_XP} experience and {REWARD_GOLD} gold, or use @quest claim."
            ),
            Some(progress) => format!(
                "Training creatures defeated: {}/{GOAL}. Equip a starter weapon from your bag and defeat Idol Spawn. Return to a training mentor after {GOAL} kills.", progress.killed
            ),
        }
    }
    pub fn ready(&self) -> bool {
        self.training
            .is_some_and(|state| state.killed == GOAL && !state.claimed)
    }
    pub fn apply(&self, command: &Command) -> Result<Self, QuestError> {
        if self
            .training
            .is_some_and(|state| state.killed > GOAL || state.claimed && state.killed != GOAL)
            || self.hunt > 2
            || self.hunt_kills > HUNT_GOAL
            || (self.hunt == 0 && self.hunt_kills != 0)
            || (self.hunt == 2 && self.hunt_kills != HUNT_GOAL)
        {
            return Err(QuestError::InvalidRecord);
        }
        let mut next = self.clone();
        match command {
            Command::AcceptPotion => {
                if self.supplies != Some(true) { return Err(QuestError::NotReady); }
                if self.potion != 0 { return Err(QuestError::AlreadyAccepted); }
                next.potion = 1;
            }
            Command::CraftedPotion => {
                if self.potion != 1 { return Err(QuestError::NotReady); } next.potion = 2;
            }
            Command::ClaimPotion => {
                if self.potion != 2 { return Err(if self.potion == 3 { QuestError::AlreadyCompleted } else { QuestError::NotReady }); } next.potion = 3;
            }
            Command::AcceptJourney => {
                if self.potion != 3 { return Err(QuestError::NotReady); }
                if self.journey != 0 { return Err(QuestError::AlreadyAccepted); } next.journey = 1;
            }
            Command::BoundLifestone => {
                if self.journey != 1 { return Err(QuestError::NotReady); } next.journey = 2;
            }
            Command::Travelled => {
                if self.journey != 2 { return Err(QuestError::NotReady); } next.journey = 3;
            }
            Command::ClaimJourney => {
                if self.journey != 3 { return Err(if self.journey == 4 { QuestError::AlreadyCompleted } else { QuestError::NotReady }); } next.journey = 4;
            }
            Command::AcceptSkill => {
                if self.journey != 4 { return Err(QuestError::NotReady); }
                if self.skill != 0 { return Err(QuestError::AlreadyAccepted); } next.skill = 1;
            }
            Command::UsedSkill => {
                if self.skill != 1 { return Err(QuestError::NotReady); } next.skill = 2;
            }
            Command::ClaimSkill => {
                if self.skill != 2 { return Err(if self.skill == 3 { QuestError::AlreadyCompleted } else { QuestError::NotReady }); } next.skill = 3;
            }
            Command::AcceptHunt => {
                if self.skill != 3 { return Err(QuestError::NotReady); }
                if self.hunt != 0 {
                    return Err(if self.hunt == 2 { QuestError::AlreadyCompleted } else { QuestError::AlreadyAccepted });
                }
                next.hunt = 1;
            }
            Command::ClaimHunt => {
                if self.hunt != 1 || self.hunt_kills != HUNT_GOAL {
                    return Err(if self.hunt == 2 { QuestError::AlreadyCompleted } else { QuestError::NotReady });
                }
                next.hunt = 2;
            }
            Command::AcceptSupplies => {
                if !self.training.is_some_and(|state| state.claimed) { return Err(QuestError::NotReady); }
                if self.supplies.is_some() { return Err(QuestError::AlreadyAccepted); }
                next.supplies = Some(false);
            }
            Command::ClaimSupplies => {
                match self.supplies {
                    None => return Err(QuestError::NotAccepted),
                    Some(true) => return Err(QuestError::AlreadyCompleted),
                    Some(false) => next.supplies = Some(true),
                }
            }
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
        // Any creature counts, including the training idol, once the hunt is open.
        if next.hunt == 1 && next.hunt_kills < HUNT_GOAL && entity >> 24 == 0x47 {
            next.hunt_kills += 1;
        }
        next
    }
    /// 0 hidden, 1 underway, 2 ready to claim, 3 rewarded.
    pub fn hunt_phase(&self) -> u8 {
        match self.hunt {
            0 => 0,
            2 => 3,
            1 if self.hunt_kills >= HUNT_GOAL => 2,
            _ => 1,
        }
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
        word(&mut payload, u32::from(self.training.is_some()) + u32::from(self.supplies.is_some()) + u32::from(self.potion != 0) + u32::from(self.journey != 0) + u32::from(self.skill != 0) + u32::from(self.hunt != 0));
        if let Some(state) = self.training {
            payload.extend(info_payload(state, false)?);
        }
        if let Some(claimed) = self.supplies { payload.extend(supplies_payload(claimed, false)?); }
        if self.potion != 0 { payload.extend(self.lesson_payload(false, false)?); }
        if self.journey != 0 { payload.extend(self.lesson_payload(true, false)?); }
        if self.skill != 0 { payload.extend(self.skill_payload(false)?); }
        if self.hunt != 0 { payload.extend(self.hunt_payload(false)?); }
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
        "Defeat three Idol Spawn training creatures, then speak to a training mentor for {REWARD_XP} experience and {REWARD_GOLD} gold. You may also use @quest claim."
    ))?);
    word(&mut bytes, ICON);
    word(&mut bytes, status(state));
    word(&mut bytes, if state.claimed { 0 } else { 1 });
    let entry = Book { training: Some(state), ..Book::default() }.guidance();
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

impl Book {
    pub fn supplies(&self) -> Option<bool> { self.supplies }
    pub fn supplies_guidance(&self) -> String {
        if self.supplies == Some(true) {
            "Field Supplies complete. You received a health potion, 80 experience and 6 gold. Use the crafting panel to combine a Mucor Mushroom and 100 Tallow into a health or vigor potion. Speak to a mentor again to begin Potion Practice.".into()
        } else {
            "Field Supplies: bring one Mucor Mushroom and one full 100-unit Tallow bundle to a training mentor. Buy them from the quartermaster for 2 gold each and keep them in your bag. Your reward is a health potion, 80 experience and 6 gold.".into()
        }
    }
    pub fn updates(&self) -> Result<Vec<Vec<u8>>, WorldError> {
        let mut events = Vec::new();
        if self.training.is_some() { events.push(self.update()?); }
        if let Some(claimed) = self.supplies {
            let mut args = script::integer(0);
            args.extend(native_graph(0xa0, &supplies_payload(claimed, true)?));
            args.extend(script::integer(if claimed { COMPLETED } else { UNDERWAY }));
            events.push(script::private_call(UPDATE, &args)?);
        }
        for journey in [false, true] {
            if if journey { self.journey != 0 } else { self.potion != 0 } {
                let mut args = script::integer(0);
                args.extend(native_graph(0xa0, &self.lesson_payload(journey, true)?));
                let claimed = if journey { self.journey == 4 } else { self.potion == 3 };
                args.extend(script::integer(if claimed { COMPLETED } else { UNDERWAY }));
                events.push(script::private_call(UPDATE, &args)?);
            }
        }
        if self.skill != 0 {
            let mut args = script::integer(0);
            args.extend(native_graph(0xa0, &self.skill_payload(true)?));
            args.extend(script::integer(if self.skill == 3 { COMPLETED } else { UNDERWAY }));
            events.push(script::private_call(UPDATE, &args)?);
        }
        if self.hunt != 0 {
            let mut args = script::integer(0);
            args.extend(native_graph(0xa0, &self.hunt_payload(true)?));
            args.extend(script::integer(if self.hunt == 2 { COMPLETED } else { UNDERWAY }));
            events.push(script::private_call(UPDATE, &args)?);
        }
        Ok(events)
    }
}
fn supplies_payload(claimed: bool, play_fx: bool) -> Result<Vec<u8>, WorldError> {
    let mut bytes = Vec::new();
    word(&mut bytes, SUPPLIES_QUEST);
    bytes.extend(world::literal_name("Field Supplies")?);
    bytes.extend(world::literal_name("Bring a Mucor Mushroom and a full 100-unit Tallow bundle to a training mentor. The mentor keeps these supplies and gives you one health potion, 80 experience and 6 gold.")?);
    word(&mut bytes, ICON);
    word(&mut bytes, if claimed { COMPLETED } else { UNDERWAY });
    word(&mut bytes, u32::from(!claimed));
    bytes.extend(world::literal_name(&Book { supplies: Some(claimed), ..Book::default() }.supplies_guidance())?);
    word(&mut bytes, 1);
    bytes.extend((-1.0f64).to_le_bytes()); bytes.extend((-1.0f64).to_le_bytes());
    word(&mut bytes, 0); bytes.extend(0f64.to_le_bytes()); bytes.extend(0f64.to_le_bytes());
    word(&mut bytes, u32::from(play_fx));
    Ok(bytes)
}
/// Canonical once-only supply exchange. Only owned bag items with the exact
/// supported profiles qualify; bought or otherwise acquired supplies are accepted.
pub fn supplies_items(bag: &crate::inventory::Inventory, first: u64) -> io::Result<Vec<crate::inventory::Command>> {
    use crate::inventory::{Command as ItemCommand, Location};
    let mushroom = bag.items().find(|item| matches!(item.location, Location::Bag(_)) && item.quantity == 1 && item.definition == crate::crafting::MUCOR && item.visual == crate::merchants::MUCOR_VISUAL)
        .ok_or_else(|| io::Error::other("Bring one Mucor Mushroom in your bag."))?;
    let tallow = bag.items().find(|item| matches!(item.location, Location::Bag(_)) && item.quantity == crate::crafting::TALLOW_QUANTITY && item.definition == crate::crafting::TALLOW)
        .ok_or_else(|| io::Error::other("Bring a full 100-unit Tallow bundle in your bag."))?;
    let mut commands = vec![ItemCommand::ConsumeDense { item: mushroom.id }, ItemCommand::ConsumeStackDense { item: tallow.id, quantity: crate::crafting::TALLOW_QUANTITY }];
    let mut candidate = bag.clone();
    for command in &commands { candidate = candidate.apply(command).map_err(io::Error::other)?; }
    let slot = candidate.free_slot().ok_or_else(|| io::Error::other("Make room for the reward potion."))?;
    commands.push(ItemCommand::Grant(crate::item_services::potion(first, slot)));
    Ok(commands)
}
/// Used both before writing and on journal replay. An arbitrary inventory
/// transaction cannot claim the objective or inject a different payout.
pub fn validate_supplies_exchange(book: &Book, bag: &crate::inventory::Inventory, items: &[crate::inventory::Command], changes: &[crate::progression::Command], first: u64) -> io::Result<Book> {
    let next = book.apply(&Command::ClaimSupplies).map_err(io::Error::other)?;
    if crate::player_transaction::encode_items(items) != crate::player_transaction::encode_items(&supplies_items(bag, first)?)
        || crate::player_transaction::encode_changes(changes) != crate::player_transaction::encode_changes(&rewards(&Command::ClaimSupplies)) {
        return Err(io::Error::other("Invalid Field Supplies exchange."));
    }
    Ok(next)
}

impl Book {
    pub fn potion_phase(&self) -> u8 { self.potion }
    pub fn journey_phase(&self) -> u8 { self.journey }
    pub fn potion_guidance(&self) -> String {
        match self.potion {
            2 => "Potion Practice: you crafted a potion. Return to a training mentor for 80 experience and 8 gold. You keep your potion.".into(),
            3 => "Potion Practice complete. You received 80 experience and 8 gold. Speak to a mentor again to begin A Safe Journey.".into(),
            _ => "Potion Practice: buy a Mucor Mushroom and a 100-unit Tallow bundle, then use the crafting panel to create a health or vigor potion. Bought potions do not count. Return to a mentor after crafting for 80 experience and 8 gold.".into(),
        }
    }
    pub fn journey_guidance(&self) -> String {
        match self.journey {
            2 => "A Safe Journey: your lifestone is bound. Leave combat and use a training portal, then speak to a training mentor at your destination.".into(),
            3 => "A Safe Journey: you bound a lifestone and used a training portal. Speak to a training mentor for 80 experience and 10 gold.".into(),
            4 => "A Safe Journey complete. You received 80 experience and 10 gold. Speak to a mentor again to begin Skill Practice.".into(),
            _ => "A Safe Journey: use a lifestone to bind it, then use a training portal. Leave combat before both actions. Speak to a mentor at your destination for 80 experience and 10 gold. Rebind now even if you bound a lifestone before accepting this lesson.".into(),
        }
    }
    fn lesson_payload(&self, journey: bool, play_fx: bool) -> Result<Vec<u8>, WorldError> {
        let (id, name, text, claimed) = if journey {
            (JOURNEY_QUEST, "A Safe Journey", self.journey_guidance(), self.journey == 4)
        } else { (POTION_QUEST, "Potion Practice", self.potion_guidance(), self.potion == 3) };
        let mut bytes = Vec::new(); word(&mut bytes, id);
        bytes.extend(world::literal_name(name)?); bytes.extend(world::literal_name(&text)?);
        word(&mut bytes, ICON); word(&mut bytes, if claimed { COMPLETED } else { UNDERWAY });
        word(&mut bytes, u32::from(!claimed)); bytes.extend(world::literal_name(&text)?); word(&mut bytes, 1);
        bytes.extend((-1.0f64).to_le_bytes()); bytes.extend((-1.0f64).to_le_bytes()); word(&mut bytes, 0);
        bytes.extend(0f64.to_le_bytes()); bytes.extend(0f64.to_le_bytes()); word(&mut bytes, u32::from(play_fx));
        Ok(bytes)
    }
}
impl Book {
    pub fn skill_phase(&self) -> u8 { self.skill }
    pub fn skill_guidance(&self) -> String {
        match self.skill {
            2 => format!("Skill Practice: you used an active skill. Leave combat and speak to a training mentor for {SKILL_XP} experience and {SKILL_GOLD} gold."),
            3 => format!("Skill Practice complete. You received {SKILL_XP} experience and {SKILL_GOLD} gold. Speak to a training mentor, or use @quest hunt, to begin Open Hunt: defeat {HUNT_GOAL} creatures anywhere."),
            _ => format!("Skill Practice: open the skill panel, train an active skill with your skill credits, drag it to your hotbar and use it. A healing skill on yourself, an attack skill on an Idol Spawn, or the Recall skill you gained by binding a lifestone all count. Then speak to a training mentor for {SKILL_XP} experience and {SKILL_GOLD} gold."),
        }
    }
    fn skill_payload(&self, play_fx: bool) -> Result<Vec<u8>, WorldError> {
        let text = self.skill_guidance();
        let claimed = self.skill == 3;
        let mut bytes = Vec::new(); word(&mut bytes, SKILL_QUEST);
        bytes.extend(world::literal_name("Skill Practice")?); bytes.extend(world::literal_name(&text)?);
        word(&mut bytes, ICON); word(&mut bytes, if claimed { COMPLETED } else { UNDERWAY });
        word(&mut bytes, u32::from(!claimed)); bytes.extend(world::literal_name(&text)?); word(&mut bytes, 1);
        bytes.extend((-1.0f64).to_le_bytes()); bytes.extend((-1.0f64).to_le_bytes()); word(&mut bytes, 0);
        bytes.extend(0f64.to_le_bytes()); bytes.extend(0f64.to_le_bytes()); word(&mut bytes, u32::from(play_fx));
        Ok(bytes)
    }
    pub fn hunt_guidance(&self) -> String {
        match self.hunt {
            2 => format!("Open Hunt complete. You defeated {HUNT_GOAL} creatures and received {HUNT_XP} experience and {HUNT_GOLD} gold. Keep training skills, crafting potions, and use /all to talk to everyone online."),
            1 if self.hunt_kills >= HUNT_GOAL => format!("Open Hunt: {HUNT_GOAL}/{HUNT_GOAL} creatures defeated. Leave combat and speak to a training mentor, or use @quest hunt claim, for {HUNT_XP} experience and {HUNT_GOLD} gold."),
            1 => format!("Open Hunt: {}/{HUNT_GOAL} creatures defeated. Any creature counts, inside or outside the training yard. Return to a mentor when you finish.", self.hunt_kills),
            _ => "Speak to a training mentor after Skill Practice to begin Open Hunt.".into(),
        }
    }
    fn hunt_payload(&self, play_fx: bool) -> Result<Vec<u8>, WorldError> {
        let text = self.hunt_guidance();
        let claimed = self.hunt == 2;
        let mut bytes = Vec::new();
        word(&mut bytes, HUNT_QUEST);
        bytes.extend(world::literal_name("Open Hunt")?);
        bytes.extend(world::literal_name(&text)?);
        word(&mut bytes, ICON);
        word(&mut bytes, if claimed { COMPLETED } else { UNDERWAY });
        word(&mut bytes, u32::from(!claimed));
        bytes.extend(world::literal_name(&text)?);
        word(&mut bytes, 2);
        bytes.extend((-1.0f64).to_le_bytes());
        bytes.extend((-1.0f64).to_le_bytes());
        word(&mut bytes, 0);
        bytes.extend(0f64.to_le_bytes());
        bytes.extend(0f64.to_le_bytes());
        word(&mut bytes, u32::from(play_fx));
        Ok(bytes)
    }
}
pub fn is_training_portal(id: u64) -> bool {
    matches!(id, 0x8140000000030002 | 0x8140000000030004 | 0x8140000000030005 | 0x8140000000030022 | 0x8140000000030025 | 0x8140000000030030 | 0x8140000000030031)
}
/// The a3 revision credits only a genuine canonical crafting exchange. Shop
/// purchases, potion grants and reward exchanges cannot advance this lesson.
pub fn validate_practice_craft(book: &Book, bag: &crate::inventory::Inventory, items: &[crate::inventory::Command], changes: &[crate::progression::Command], first: u64) -> io::Result<Book> {
    use crate::inventory::Command as ItemCommand;
    let bad = || io::Error::other("Invalid Potion Practice craft transaction.");
    let [ItemCommand::ConsumeDense { item: mushroom }, ItemCommand::ConsumeStackDense { item: tallow, quantity }, ItemCommand::Grant(output)] = items else { return Err(bad()); };
    if *quantity != crate::crafting::TALLOW_QUANTITY || !changes.is_empty() { return Err(bad()); }
    let recipe = match output.definition { crate::crafting::HEALTH_OUTPUT => crate::crafting::HEALTH_RECIPE, crate::crafting::VIGOR_OUTPUT => crate::crafting::VIGOR_RECIPE, _ => return Err(bad()) };
    let request = crate::crafting::ExecuteRequest { primary: *mushroom, ingredients: std::collections::BTreeMap::from([(1, *mushroom), (2, *tallow)]), spinner: 0, recipe };
    request.require_inputs(bag, true).map_err(|_| bad())?;
    let candidate = bag.apply(&items[0]).and_then(|bag| bag.apply(&items[1])).map_err(|_| bad())?;
    let expected = crate::crafting::output(recipe, first, candidate.free_slot().ok_or_else(bad)?).ok_or_else(bad)?;
    if output != &expected { return Err(bad()); }
    book.apply(&Command::CraftedPotion).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trained() -> Book {
        let mut book = Book::default();
        for command in [
            Command::Accept,
            Command::Kill(TARGET_ENTITY),
            Command::Kill(TARGET_ENTITY),
            Command::Kill(TARGET_ENTITY),
            Command::Claim,
            Command::AcceptSupplies,
            Command::ClaimSupplies,
            Command::AcceptPotion,
            Command::CraftedPotion,
            Command::ClaimPotion,
            Command::AcceptJourney,
            Command::BoundLifestone,
            Command::Travelled,
            Command::ClaimJourney,
            Command::AcceptSkill,
            Command::UsedSkill,
            Command::ClaimSkill,
        ] {
            book = book.apply(&command).unwrap();
        }
        book
    }

    #[test]
    fn open_hunt_counts_any_creature_and_ignores_kills_before_it_starts() {
        let mut book = Book::default();
        book = book.apply(&Command::Accept).unwrap();
        let untouched = book.apply(&Command::Kill(0x47000011)).unwrap();
        assert_eq!(untouched, book);
        assert!(book.apply(&Command::AcceptHunt).is_err());

        let mut book = trained();
        assert!(book.guidance().contains("Open Hunt"));
        book = book.apply(&Command::AcceptHunt).unwrap();
        assert!(book.apply(&Command::AcceptHunt).is_err());
        assert!(book.apply(&Command::ClaimHunt).is_err());
        // Training idols already claimed do not move the old lesson, but they do count here.
        book = book.apply(&Command::Kill(TARGET_ENTITY)).unwrap();
        book = book.apply(&Command::Kill(0x47001234)).unwrap();
        assert_eq!(book.hunt_phase(), 1);
        assert!(book.guidance().contains("2/8"));
        for _ in 0..6 {
            book = book.apply(&Command::Kill(0x47000099)).unwrap();
        }
        assert_eq!(book.hunt_phase(), 2);
        let full = book.apply(&Command::Kill(0x47000099)).unwrap();
        assert_eq!(full, book);
        book = book.apply(&Command::ClaimHunt).unwrap();
        assert_eq!(book.hunt_phase(), 3);
        assert!(book.apply(&Command::ClaimHunt).is_err());
        assert!(book.list_graph().is_ok());
        assert!(book.updates().unwrap().len() >= 6);
        assert_eq!(Command::decode("h.a").unwrap(), Command::AcceptHunt);
        assert_eq!(Command::decode("h.c").unwrap(), Command::ClaimHunt);
    }
}
