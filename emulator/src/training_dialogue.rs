// SPDX-License-Identifier: AGPL-3.0-only
//! Authored training conversations, separate from recovered native wire layouts.
use crate::{appearance, quests::Book};

pub const MENTOR_ENTITY: u32 = 0x47001699;
pub const TUMEROK_RETURN: u64 = 0x8140000000030030;
pub const LUGIAN_RETURN: u64 = 0x8140000000030031;

pub fn race_name(race: u32) -> &'static str {
    match race { appearance::LUGIAN => "Lugian", appearance::TUMEROK => "Tumerok", _ => "Human" }
}
pub fn greeting(race: u32, book: &Book) -> String {
    format!("Welcome, {} adventurer. {} Ask @mentor help while nearby for lessons on gear, combat, healing, crafting, travel and skills.", race_name(race), book.guidance())
}
pub fn topic(value: &str, race: u32, book: &Book) -> Option<String> {
    Some(match value.trim().to_ascii_lowercase().as_str() {
        "" | "help" => "I can help with gear, combat, healing, supplies, crafting, travel and skills. Ask @mentor followed by one of those topics. Use me normally to accept the next lesson or deliver a completed objective. Your journal keeps all five lessons across logins.".into(),
        "skill" | "skills" => "Each level grants skill credits. Open the skill panel, train an active skill your race can learn, then drag it to your hotbar. Healing skills can target yourself; attack skills need a selected creature in range and the right weapon in hand. Spend experience in the skill panel to raise trained skills.".into(),
        "quest" | "quests" => book.guidance(),
        "gear" => format!("Your {} starter weapons are in your bag and your starter clothing is already worn. Drag a weapon to its equipment slot, or use it from your bag. Drag it back into your bag to unequip. Keep your racial starter gear; the quartermaster also sells a development training weapon.", race_name(race)),
        "combat" => "Equip a starter weapon, select an Idol Spawn, and use your attack or a trained starter attack skill. Stay in range and watch your health and vigor. Defeat three Idol Spawn for Training Grounds, leave combat, then use me to claim the reward.".into(),
        "healing" => "Leave combat before using a health or vigor potion from your bag. Health potions restore health; vigor potions restore vigor. Use a nearby lifestone while alive and out of combat to bind a recall point. If defeated, use the resurrection control before continuing your lessons.".into(),
        "supplies" => book.supplies_guidance(),
        "crafting" => "The quartermaster sells Mucor Mushrooms and 100-unit Tallow bundles for 2 gold each. The crafting panel offers health and vigor potion recipes. Put one mushroom and a full Tallow bundle in your bag, choose the recipe, assign both ingredients and craft while out of combat. Field Supplies delivery uses its own pair of ingredients.".into(),
        "travel" => "The nearby training portal links your start to the shared training area. The Human room and corridor have exits to the outdoor hub. Examine the hub portals to see their destinations: Human training room, Tumerok start or Lugian start. Leave combat and stand near a portal before using it.".into(),
        _ => return None,
    })
}
pub fn portal_name(id: u64) -> Option<&'static str> {
    match id {
        TUMEROK_RETURN => Some("Return to Tumerok Training"),
        LUGIAN_RETURN => Some("Return to Lugian Training"),
        0x8140000000030005 => Some("Return to Human Training"),
        0x8140000000030022 | 0x8140000000030025 => Some("Shared Training Grounds"),
        _ => None,
    }
}
