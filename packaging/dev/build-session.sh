#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
target=${AB_SESSION_TARGET:-${CARGO_TARGET_DIR:-$root/rust/target}}
CARGO_TARGET_DIR=$target cargo build --manifest-path "$root/rust/Cargo.toml" --release --locked -p ab-session >&2
binary=$target/release/ab-session
[[ -x $binary ]]
printf '%s\n' "$binary"
