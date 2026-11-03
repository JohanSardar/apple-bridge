#!/usr/bin/env bash
set -euo pipefail
if [[ $(uname -s) != Linux ]]; then
  echo 'gateway-matrix requires Linux; sync this checkout and run make gateway-matrix on ssh build' >&2
  exit 1
fi
go build -tags lab -o bin/bridge ./cmd/bridge
go build -o bin/bridge-lab ./cmd/bridge-lab
bin/bridge-lab build-legacy
binary=${RUST_BRIDGE:-}
if [[ -z "$binary" ]]; then
  artifacts=$(mktemp)
  trap 'rm -f "$artifacts"' EXIT
  if ! cargo build --manifest-path rust/Cargo.toml -p ab-runtime --features lab-queries --bin bridge --message-format=json > "$artifacts"; then
    echo "BLOCKED ab-runtime: Rust gateway build unavailable; reporting every default scenario" >&2
  fi
  binary=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="bridge" and m.get("executable")]; print(paths[-1] if paths else "")' < "$artifacts")
fi
selection=()
if [[ -n "${SIM_SELECT:-}" ]]; then
  selection=(--select "$SIM_SELECT")
fi
python3 lab/testing/sim_parallel.py --parallel "${SIM_JOBS:-4}" --timeout "${SIM_TIMEOUT:-900}" --output "${SIM_OUTPUT:?}" --impl gateway=rust --rust-binary "$binary" "${selection[@]}"
