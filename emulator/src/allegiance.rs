// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Vanquish
//! Native allegiance profile queries and a constructor-default unaffiliated reply.
//!
//! This is not an allegiance hierarchy, swearing, patronage or XP system. The
//! caller must resolve the self sentinel, authorize the requested player, and
//! establish that the target has no allegiance before sending this reply.
//! Matching build-7148 WLib 56000005: QueryAllegianceProfile b400b3 at 41ba4
//! calls sender 82030036 at fe084. Reply 8203002e reaches 1aa40129 -> 81b6004c
//! -> 01b50002 -> native UpdateAllegianceProfile (0046a3f0).
use crate::{script, world::WorldError};

pub const QUERY: u32 = 0x82030005;
pub const PROFILE: u32 = 0x8203002e;
/// Verify this interpreted field count against the active matching WLib.
pub const LAYOUTS: [(usize, u32); 1] = [(0x1b5, 13)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    /// None is the native automatic self-profile request, observed on login.
    /// Some requests a player profile, not authority to change that player.
    pub target: Option<u64>,
}
impl Query {
    pub fn decode(arguments: &[u8]) -> Result<Self, WorldError> {
        if arguments.len() != 12 || u32::from_le_bytes(arguments[..4].try_into().unwrap()) != 2 {
            return Err(WorldError::InvalidValue);
        }
        let target = u64::from_le_bytes(arguments[4..12].try_into().unwrap());
        if target == u64::MAX {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            target: (target != 0).then_some(target),
        })
    }
}

/// Fresh 01b5 state with the exact 81b50010 constructor defaults (b1764).
/// The adapter at b2778 guards every absent node/list/name/officer reference.
/// Native AllegianceProfileAdaptor constructor 0060ca40 leaves member identity
/// INVALID_IID through 0060c6f0; gmAllegianceUI::Update 0043d790 then selects
/// token_noallegiance and returns before walking a hierarchy.
///
/// Captured browsing.allegiance.tree.elog corroborates the thirteen field tags,
/// but its populated character graphs are not copied. Native acceptance of
/// this newly encoded constructor-only profile remains unverified in-game.
pub fn unaffiliated() -> Result<Vec<u8>, WorldError> {
    let mut arguments = Vec::with_capacity(96);
    for word in [5u32, 1, 1, 0x44000100, 0, 0x01b50000, 13] {
        arguments.extend(word.to_le_bytes());
    }
    for field in 0..13 {
        let value: u32 = if [0, 1, 2, 3, 6, 7, 8, 12].contains(&field) {
            u32::MAX
        } else {
            0
        };
        arguments.extend(value.to_le_bytes());
    }
    arguments.extend([1u8, 1, 1, 1, 4, 4, 1, 1, 1, 4, 2, 4, 1]);
    while arguments.len() % 4 != 0 {
        arguments.push(0);
    }
    script::private_call(PROFILE, &arguments)
}
