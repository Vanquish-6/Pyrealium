#!/bin/sh
# Build on a separate Linux/WSL machine, never on the shared production host.
# Requires rustup's aarch64-unknown-linux-musl target; no C toolchain needed.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target=aarch64-unknown-linux-musl
host=$(rustc -vV | sed -n 's/^host: //p')
sysroot=$(rustc --print sysroot)
linker="$sysroot/lib/rustlib/$host/bin/rust-lld"
[ -x "$linker" ] || { echo 'Rust LLD linker is missing.' >&2; exit 1; }
: "${PYREALIUM_SOURCE_URL:?Set the URL of the exact published source revision}"
export PYREALIUM_SOURCE_URL
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$linker"
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
cargo build --locked --release --target "$target" --manifest-path "$root/emulator/Cargo.toml"
printf 'Built %s\n' "$root/emulator/target/$target/release/pyrealium"
