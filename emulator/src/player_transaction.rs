//! Candidate validation shared by durable inventory/progression transactions.
use crate::{inventory, progression};
use std::io;

fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}

pub fn candidate(
    inventory: &inventory::Inventory,
    progression: &progression::Progression,
    items: &[inventory::Command],
    changes: &[progression::Command],
    first_item: Option<u64>,
) -> io::Result<(inventory::Inventory, progression::Progression)> {
    if items.len() > 8 || changes.len() > 8 || items.is_empty() && changes.is_empty() {
        return Err(invalid("player transaction command count"));
    }
    let mut next_item = first_item;
    let mut bag = inventory.clone();
    for command in items {
        if let Some(allocated) = command.allocated_id() {
            if next_item != Some(allocated) || allocated >> 48 != inventory::ITEM_NAMESPACE >> 48 {
                return Err(invalid("nonsequential transaction item identity"));
            }
            next_item = allocated.checked_add(1);
        }
        bag = bag.apply(command).map_err(io::Error::other)?;
    }
    let mut state = progression.clone();
    for command in changes {
        state = state.apply(command).map_err(io::Error::other)?;
    }
    Ok((bag, state))
}

pub fn encode_items(items: &[inventory::Command]) -> String {
    if items.is_empty() {
        "-".into()
    } else {
        items
            .iter()
            .map(inventory::Command::encode)
            .collect::<Vec<_>>()
            .join(";")
    }
}
pub fn encode_changes(changes: &[progression::Command]) -> String {
    if changes.is_empty() {
        "-".into()
    } else {
        changes
            .iter()
            .map(progression::Command::encode)
            .collect::<Vec<_>>()
            .join(";")
    }
}
pub fn decode_items(text: &str) -> io::Result<Vec<inventory::Command>> {
    if text == "-" {
        return Ok(Vec::new());
    }
    if text.len() > 2048 || text.split(';').count() > 8 {
        return Err(invalid("transaction inventory encoding limit"));
    }
    text.split(';')
        .map(|entry| inventory::Command::decode(entry).map_err(io::Error::other))
        .collect()
}
pub fn decode_changes(text: &str) -> io::Result<Vec<progression::Command>> {
    if text == "-" {
        return Ok(Vec::new());
    }
    if text.len() > 512 || text.split(';').count() > 8 {
        return Err(invalid("transaction progression encoding limit"));
    }
    text.split(';')
        .map(|entry| progression::Command::decode(entry).map_err(io::Error::other))
        .collect()
}
