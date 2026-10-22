#!/bin/bash
set -uo pipefail
python3 "$(dirname "$0")/signed-app-cache.py" --trim || true
threshold_kb=${APPLE_BRIDGE_MIN_FREE_KB:-15728640}
# Trim with 1 GiB of headroom so concurrent jobs don't push the host below the floor before the next check.
headroom_kb=${APPLE_BRIDGE_TRIM_HEADROOM_KB:-1048576}
free_kb=$(df -k / | awk 'NR==2{print $4}')
[ "$free_kb" -ge $((threshold_kb + headroom_kb)) ] && exit 0
cache=$(go env GOCACHE 2>/dev/null)
if [ -n "$cache" ] && [ -d "$cache" ]; then
  find "$cache" -type f -size +20M -mmin +60 -delete 2>/dev/null
  find "$cache" -type f -mmin +360 -delete 2>/dev/null
  find "$cache" -type d -empty -delete 2>/dev/null
fi
find /tmp/ablab -mindepth 1 -maxdepth 1 -type d -mmin +360 -exec rm -rf {} + 2>/dev/null
# go run/go test leave go-build*/go-link-* work directories behind when interrupted. Remove ones older than 2 hours
# that no running process executes from.
temp=$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || echo "${TMPDIR:-/tmp}/")
for dir in "$temp"go-build* "$temp"go-link-*; do
  [ -d "$dir" ] || continue
  [ -n "$(find "$dir" -maxdepth 0 -mmin +120 2>/dev/null)" ] || continue
  pgrep -f "$dir/" >/dev/null 2>&1 && continue
  rm -rf "$dir"
done
echo "trim-disk: free $(df -h / | awk 'NR==2{print $4}') after trimming" >&2
