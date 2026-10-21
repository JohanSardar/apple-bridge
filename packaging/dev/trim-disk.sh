#!/bin/bash
set -uo pipefail
threshold_kb=${APPLE_BRIDGE_MIN_FREE_KB:-5242880}
free_kb=$(df -k / | awk 'NR==2{print $4}')
[ "$free_kb" -ge "$threshold_kb" ] && exit 0
cache=$(go env GOCACHE 2>/dev/null)
if [ -n "$cache" ] && [ -d "$cache" ]; then
  find "$cache" -type f -mmin +360 -delete 2>/dev/null
  find "$cache" -type d -empty -delete 2>/dev/null
fi
find /tmp/ablab -mindepth 1 -maxdepth 1 -type d -mmin +360 -exec rm -rf {} + 2>/dev/null
echo "trim-disk: free $(df -h / | awk 'NR==2{print $4}') after trimming" >&2
