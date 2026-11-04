#!/bin/bash
set -uo pipefail
python3 "$(dirname "$0")/signed-app-cache.py" --trim || true
threshold_kb=${APPLE_BRIDGE_MIN_FREE_KB:-15728640}
# Trim with 1 GiB of headroom so concurrent jobs don't push the host below the floor before the next check.
headroom_kb=${APPLE_BRIDGE_TRIM_HEADROOM_KB:-1048576}
free_kb=$(df -k / | awk 'NR==2{print $4}')
[ "$free_kb" -ge $((threshold_kb + headroom_kb)) ] && exit 0
target=${CARGO_TARGET_DIR:-}
if [ -n "$target" ] && [ -d "$target" ]; then
  # Incremental compilation state is rebuilt on demand and grows by gigabytes across worker branches.
  find "$target" -type d -name incremental -prune -mmin +120 -exec rm -rf {} + 2>/dev/null
  [ "$(df -k / | awk 'NR==2{print $4}')" -lt $((threshold_kb + headroom_kb)) ] && find "$target" -type d -name incremental -prune -mmin +60 -exec rm -rf {} + 2>/dev/null
fi
find /tmp/ablab -mindepth 1 -maxdepth 1 -type d -mmin +360 -exec rm -rf {} + 2>/dev/null
echo "trim-disk: free $(df -h / | awk 'NR==2{print $4}') after trimming" >&2
