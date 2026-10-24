// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Vanquish
//! Public license/source notice for the server and its remote users.
pub const SOURCE_URL: &str = env!("PYREALIUM_COMPILED_SOURCE_URL");
pub const LICENSE_TEXT: &str = include_str!("../../LICENSE");

pub fn notice() -> String {
    format!(
        "Pyrealium Copyright (C) 2026 Vanquish. AGPL-3.0-only; no warranty. You may redistribute under that license. Source and license: {SOURCE_URL}"
    )
}
