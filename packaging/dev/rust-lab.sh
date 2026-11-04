#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
manifest=rust/Cargo.toml

artifact() {
  local name=$1 out
  shift
  out=$(mktemp)
  cargo build --manifest-path "$manifest" --locked "$@" --message-format=json > "$out"
  python3 -c 'import json,sys
name=sys.argv[1]
paths=[m["executable"] for line in sys.stdin if line.startswith("{") and (m:=json.loads(line)).get("reason")=="compiler-artifact" and m.get("target",{}).get("name")==name and m.get("executable")]
print(paths[-1] if paths else "")' "$name" < "$out"
  rm -f "$out"
}

require() {
  if [[ -z "$2" || ! -x "$2" ]]; then
    echo "rust-lab: could not build $1" >&2
    exit 1
  fi
}

prepare() {
  BRIDGE=${RUST_BRIDGE:-$(artifact bridge -p ab-runtime --features lab-queries --bin bridge)}
  FAMILIES=${LAB_FAMILIES:-$(artifact lab-families -p lab-adapters --bin lab-families)}
  SIMAPPLE=${LAB_SIMAPPLE:-$(artifact lab-simapple -p lab-simapple --bin lab-simapple)}
  require bridge "$BRIDGE"
  require lab-families "$FAMILIES"
  require lab-simapple "$SIMAPPLE"
  if [[ -z "${AB_SESSION_BINARY:-}" ]]; then
    AB_SESSION_BINARY=$(artifact ab-session -p ab-session --features lab --bin ab-session)
  fi
  if [[ -z "${AB_RELAY_FIXTURE:-}" ]]; then
    AB_RELAY_FIXTURE=$(artifact lab_fixture -p ab-relay --examples)
    cargo build --manifest-path "$manifest" --locked -p ab-replication --example relay2_fixture
    [[ -x "$(dirname "$AB_RELAY_FIXTURE")/relay2_fixture" ]] || { echo "relay2_fixture missing next to $AB_RELAY_FIXTURE" >&2; exit 1; }
  fi
  require ab-session "$AB_SESSION_BINARY"
  require lab_fixture "$AB_RELAY_FIXTURE"
  export AB_SESSION_BINARY AB_RELAY_FIXTURE
}

scenario_files() {
  local arg
  for arg in "$@"; do
    if [[ -d "$arg" ]]; then
      find "$arg" -name '*.yaml' -type f | sort
    else
      printf '%s\n' "$arg"
    fi
  done
}

run_files() {
  local level=$1 output files=()
  shift
  while IFS= read -r file; do files+=("$file"); done < <(scenario_files "$@")
  [[ ${#files[@]} -gt 0 ]] || { echo "rust-lab: no scenarios" >&2; exit 1; }
  output=$(mktemp -d "${TMPDIR:-/tmp}/ab-rust-lab.XXXXXX")
  local extra=()
  if [[ $level == real ]]; then
    extra+=(--level real)
    [[ -n "${REAL_HELPER_APP:-}" ]] && extra+=(--helper-app "$REAL_HELPER_APP")
  fi
  "$FAMILIES" run "${files[@]}" --gateway "$BRIDGE" --simapple "$SIMAPPLE" --output "$output" ${extra[@]+"${extra[@]}"}
}

selected() {
  local arg
  for arg in "$@"; do
    [[ $arg == lab/scenarios ]] || return 0
  done
  return 1
}

command=${1:-}
[[ $# -gt 0 ]] && shift
case "$command" in
  binaries)
    prepare
    printf 'bridge=%s\nlab-families=%s\nlab-simapple=%s\nab-session=%s\nlab_fixture=%s\n' "$BRIDGE" "$FAMILIES" "$SIMAPPLE" "$AB_SESSION_BINARY" "$AB_RELAY_FIXTURE"
    ;;
  sim)
    prepare
    if [[ $# -eq 0 ]] || ! selected "$@"; then
      python3 lab/testing/sim_parallel.py --runner rust --parallel "${SIM_JOBS:-4}" --timeout "${SIM_TIMEOUT:-900}" --output "${SIM_OUTPUT:-${TMPDIR:-/tmp}/ab-rust-sim-$(date +%s)}" --rust-binary "$BRIDGE" --families-binary "$FAMILIES" --simapple-binary "$SIMAPPLE" ${SIM_SELECT:+--select "$SIM_SELECT"}
    else
      run_files sim "$@"
    fi
    ;;
  families)
    prepare
    python3 rust/lab/families-parity.py --rust-only --families "$FAMILIES" --gateway "$BRIDGE" --simapple "$SIMAPPLE" --output "${FAMILIES_OUTPUT:-${TMPDIR:-/tmp}/ab-rust-families-$(date +%s)}" "$@"
    ;;
  replica)
    prepare
    python3 lab/testing/sim_parallel.py --runner rust --level replica --parallel "${SIM_JOBS:-4}" --timeout "${SIM_TIMEOUT:-900}" --output "${SIM_OUTPUT:-${TMPDIR:-/tmp}/ab-rust-replica-$(date +%s)}" --rust-binary "$BRIDGE" --families-binary "$FAMILIES" --simapple-binary "$SIMAPPLE" ${SIM_SELECT:+--select "$SIM_SELECT"}
    ;;
  real)
    if [[ $# -eq 0 ]] || ! selected "$@"; then
      echo "rust-lab: real level needs explicit scenarios (SCENARIOS=...)" >&2
      exit 1
    fi
    prepare
    run_files real "$@"
    ;;
  agent)
    echo "rust-lab: the agent level has no Rust runner yet; lab/agent is Go only (see GO-DELETE-LIST)" >&2
    exit 1
    ;;
  *)
    echo "usage: rust-lab.sh binaries | sim [YAML...] | families [families-parity flags] | replica | real YAML... | agent" >&2
    exit 2
    ;;
esac
