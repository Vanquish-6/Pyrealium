// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Vanquish
use std::{env, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=PYREALIUM_SOURCE_URL");
    let mut refs = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        refs.push(branch);
    }
    for file in refs {
        if let Some(path) = git(&["rev-parse", "--git-path", &file]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let revision = git(&["rev-parse", "HEAD"])
        .filter(|value| value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()));
    let default = revision.map_or_else(
        || "https://github.com/Vanquish-6/Pyrealium".into(),
        |revision| format!("https://github.com/Vanquish-6/Pyrealium/tree/{revision}"),
    );
    let source = env::var("PYREALIUM_SOURCE_URL").unwrap_or(default);
    assert!(
        source.len() <= 256
            && (source.starts_with("https://") || source.starts_with("http://"))
            && source.bytes().all(|b| b.is_ascii_graphic())
            && !source.contains('@'),
        "PYREALIUM_SOURCE_URL must be a public HTTP(S) source URL, without credentials or whitespace"
    );
    println!("cargo:rustc-env=PYREALIUM_COMPILED_SOURCE_URL={source}");
}
