#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
version=${1:-$(git -C "$root" rev-parse --short=12 HEAD)}
out=${2:-$root/dist/relay}
target=x86_64-unknown-linux-musl
build_dir=${CARGO_TARGET_DIR:-$root/rust/target}
(cd "$root/rust" && cargo build --release --locked --target "$target" -p ab-relay --bin ab-relay -p ab-relay-control --bin ab-relay-control)
(cd "$root/services/accounts" && npm ci --no-audit --no-fund --silent && npm run -s build)
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
bundle=$staging/ab-relay-$version
mkdir -p "$bundle/bin" "$bundle/systemd"
cp "$build_dir/$target/release/ab-relay" "$build_dir/$target/release/ab-relay-control" "$bundle/bin/"
cp "$root/services/accounts/dist/accounts.mjs" "$bundle/"
cp "$root/deploy/selfhost/install.sh" "$root/deploy/selfhost/ab-relay-admin" "$bundle/"
cp "$root"/deploy/selfhost/systemd/*.service "$bundle/systemd/"
printf '%s\n' "$version" >"$bundle/VERSION"
mkdir -p "$out"
tar -C "$staging" -czf "$out/ab-relay-$version-linux-amd64.tar.gz" "ab-relay-$version"
(cd "$out" && sha256sum "ab-relay-$version-linux-amd64.tar.gz" >"ab-relay-$version-linux-amd64.tar.gz.sha256")
echo "$out/ab-relay-$version-linux-amd64.tar.gz"
