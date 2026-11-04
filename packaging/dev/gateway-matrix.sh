#!/usr/bin/env bash
set -euo pipefail
if [[ $(uname -s) != Linux ]]; then
  echo 'gateway-matrix requires Linux; sync this checkout and run make gateway-matrix on ssh build' >&2
  exit 1
fi
if [[ ${AB_LAB_RUNNER:-go} == rust ]]; then
  : "${SIM_OUTPUT:?}"
  exec "$(dirname "$0")/rust-lab.sh" sim
fi
go build -tags lab -o bin/bridge ./cmd/bridge
go build -o bin/bridge-lab ./cmd/bridge-lab
bin/bridge-lab build-legacy
artifacts=$(mktemp)
trap 'rm -f "$artifacts"' EXIT
binary=${RUST_BRIDGE:-}
if [[ -z "$binary" ]]; then
  if ! cargo build --manifest-path rust/Cargo.toml -p ab-runtime --features lab-queries --bin bridge --message-format=json > "$artifacts"; then
    echo "BLOCKED ab-runtime: Rust gateway build unavailable; reporting every default scenario" >&2
  fi
  binary=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="bridge" and m.get("executable")]; print(paths[-1] if paths else "")' < "$artifacts")
fi
session=${AB_SESSION_BINARY:-}
if [[ -z "$session" ]]; then
  cargo build --manifest-path rust/Cargo.toml -p ab-session --features lab --bin ab-session --message-format=json > "$artifacts"
  session=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="ab-session" and m.get("executable")]; print(paths[-1] if paths else "")' < "$artifacts")
fi
export AB_SESSION_BINARY="$session"
relay_fixture=${AB_RELAY_FIXTURE:-}
if [[ -z "$relay_fixture" ]]; then
  cargo build --manifest-path rust/Cargo.toml -p ab-relay --examples --message-format=json > "$artifacts"
  relay_fixture=$(python3 -c 'import json,sys; paths=[m["executable"] for line in sys.stdin if (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")=="lab_fixture" and m.get("executable")]; print(paths[-1] if paths else "")' < "$artifacts")
  cargo build --manifest-path rust/Cargo.toml -p ab-replication --example relay2_fixture
  relay2_fixture=$(dirname "$relay_fixture")/relay2_fixture
  [[ -x "$relay2_fixture" ]] || { echo "relay2_fixture missing next to $relay_fixture" >&2; exit 1; }
fi
export AB_RELAY_FIXTURE="$relay_fixture"
selection=()
if [[ -n "${SIM_SELECT:-}" ]]; then
  selection=(--select "$SIM_SELECT")
fi
python3 lab/testing/sim_parallel.py --parallel "${SIM_JOBS:-4}" --timeout "${SIM_TIMEOUT:-900}" --output "${SIM_OUTPUT:?}" --impl gateway=rust --rust-binary "$binary" "${selection[@]}"
