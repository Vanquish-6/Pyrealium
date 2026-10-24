//! Coalesce only uninterrupted progression runs. Other transactions are replay
//! barriers, preserving training, inventory, binding and lifetime ordering.
use super::{hash, invalid};
use crate::progression::Progression;
use std::{collections::BTreeMap, io};

#[derive(Default)]
pub(super) struct Run {
    states: BTreeMap<u64, Progression>,
    raw: Vec<String>,
}
impl Run {
    pub(super) fn record(&mut self, id: u64, state: &Progression, line: &str) {
        self.states.insert(id, state.clone());
        self.raw.push(line.to_owned());
    }
    pub(super) fn flush(&mut self, output: &mut Vec<String>) {
        if self.raw.is_empty() {
            return;
        }
        let checkpoints: Vec<_> = self
            .states
            .iter()
            .map(|(id, state)| {
                let data = format!(
                    "g3\t{id}\t{},{},{},{},{}",
                    state.xp, state.available_xp, state.gold, state.health, state.vigor
                );
                format!("{data}\t{:08x}", hash(data.as_bytes()))
            })
            .collect();
        let old: usize = self.raw.iter().map(|line| line.len() + 1).sum();
        let new: usize = checkpoints.iter().map(|line| line.len() + 1).sum();
        if new < old {
            output.extend(checkpoints);
        } else {
            output.append(&mut self.raw);
        }
        self.states.clear();
        self.raw.clear();
    }
}
pub(super) fn progression(text: &str, previous: &Progression) -> io::Result<Progression> {
    if text.len() > 128 {
        return Err(invalid("progression checkpoint size"));
    }
    let f: Vec<_> = text.split(',').collect();
    if f.len() != 5 {
        return Err(invalid("progression checkpoint shape"));
    }
    let next = Progression {
        xp: f[0].parse().map_err(|_| invalid("checkpoint XP"))?,
        available_xp: f[1]
            .parse()
            .map_err(|_| invalid("checkpoint available XP"))?,
        gold: f[2].parse().map_err(|_| invalid("checkpoint gold"))?,
        health: f[3].parse().map_err(|_| invalid("checkpoint health"))?,
        vigor: f[4].parse().map_err(|_| invalid("checkpoint vigor"))?,
    };
    if !next.valid()
        || next.xp < previous.xp
        || next.available_xp.saturating_sub(previous.available_xp) > next.xp - previous.xp
    {
        return Err(invalid("invalid progression checkpoint transition"));
    }
    Ok(next)
}
