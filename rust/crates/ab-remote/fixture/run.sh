#!/bin/sh
set -eu
cd "$(dirname "$0")/../../../.."
export CARGO_BUILD_JOBS=2
cargo build --manifest-path rust/Cargo.toml -p ab-remote --example bridge --locked
AB_REMOTE_BINARY="${CARGO_TARGET_DIR:-$PWD/rust/target}/debug/examples/bridge" go test ./rust/crates/ab-remote/fixture -v -count=1 -timeout 60s
