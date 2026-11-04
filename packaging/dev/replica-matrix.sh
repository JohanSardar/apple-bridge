#!/usr/bin/env bash
set -euo pipefail
flags=${REPLICA_MATRIX_FLAGS:---local-replicas --jobs 4}
jobs=$(printf '%s\n' "$flags" | sed -n 's/.*--jobs \([0-9][0-9]*\).*/\1/p')
if [[ -n "${SCENARIOS:-}" && "$SCENARIOS" != lab/scenarios ]]; then
  exec "$(dirname "$0")/rust-lab.sh" sim "$SCENARIOS"
fi
SIM_JOBS=${jobs:-4} exec "$(dirname "$0")/rust-lab.sh" replica
