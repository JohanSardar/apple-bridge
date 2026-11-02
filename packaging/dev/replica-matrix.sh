#!/usr/bin/env bash
set -euo pipefail
if [[ -n "${RUST_BRIDGE:-}" ]]; then
  exec go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl gateway=go,replica=rust --rust-binary "$RUST_BRIDGE" --local-replicas
fi
metadata=$(cargo metadata --manifest-path rust/Cargo.toml --format-version 1 --no-deps)
if ! python3 -c 'import json,sys; data=json.load(sys.stdin); sys.exit(0 if any(t["name"] == "bridge" and "bin" in t["kind"] for p in data["packages"] for t in p["targets"]) else 1)' <<< "$metadata"; then
  exec go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl gateway=go,replica=rust --local-replicas
fi
artifacts=$(mktemp)
trap 'rm -f "$artifacts"' EXIT
cargo build --manifest-path rust/Cargo.toml --bin bridge --message-format=json > "$artifacts"
binary=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="bridge" and m.get("executable")]; sys.exit("cargo emitted no bridge executable") if len(paths)!=1 else print(paths[0])' < "$artifacts")
go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl gateway=go,replica=rust --rust-binary "$binary" --local-replicas
