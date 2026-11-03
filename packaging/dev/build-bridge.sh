#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
output=${1:?output path}
target=${2:-aarch64-apple-darwin}
package=${3:-ab-runtime}
binary=${4:-bridge}
cargo=${CARGO:-cargo}
if [ "$(uname)" = Darwin ] && [ -z "${SDKROOT:-}" ]; then
  sdk=$("$root/packaging/dev/sdkroot.sh" || true)
  if [ -n "$sdk" ]; then
    export SDKROOT=$sdk
  fi
fi
"$cargo" build --locked --release --manifest-path "$root/rust/Cargo.toml" -p "$package" --bin "$binary" --target "$target"
built="${CARGO_TARGET_DIR:-$root/rust/target}/$target/release/$binary"
mkdir -p "$(dirname "$output")"
install -m 755 "$built" "$output"
