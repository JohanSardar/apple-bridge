#!/usr/bin/env bash
set -euo pipefail
read -r -a matrix_flags <<< "${REPLICA_MATRIX_FLAGS:---local-replicas --jobs 4}"
implementation=${REPLICA_IMPL:-gateway=go,replica=rust}
if [[ "$implementation" == "gateway=go,replica=go" ]]; then
  exec go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl "$implementation" "${matrix_flags[@]}"
fi
if [[ -n "${RUST_BRIDGE:-}" ]]; then
  exec go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl "$implementation" --rust-binary "$RUST_BRIDGE" "${matrix_flags[@]}"
fi
metadata=$(cargo metadata --manifest-path rust/Cargo.toml --format-version 1 --no-deps)
if ! python3 -c 'import json,sys; data=json.load(sys.stdin); sys.exit(0 if any(t["name"] == "bridge" and "bin" in t["kind"] for p in data["packages"] for t in p["targets"]) else 1)' <<< "$metadata"; then
  exec go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl "$implementation" "${matrix_flags[@]}"
fi
artifacts=$(mktemp)
trap 'rm -f "$artifacts"' EXIT
cargo build --manifest-path rust/Cargo.toml --bin bridge --features ab-runtime/lab-queries --message-format=json > "$artifacts"
binary=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="bridge" and m.get("executable")]; sys.exit("cargo emitted no bridge executable") if len(paths)!=1 else print(paths[0])' < "$artifacts")
go run ./cmd/bridge-lab replica-matrix "${SCENARIOS:-lab/scenarios}" --impl "$implementation" --rust-binary "$binary" "${matrix_flags[@]}"
