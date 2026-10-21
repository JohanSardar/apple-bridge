#!/bin/bash
set -euo pipefail
if [ -z "${SDKROOT:-}" ]; then SDKROOT=$("$(dirname "$0")/sdkroot.sh"); [ -n "$SDKROOT" ] && export SDKROOT || unset SDKROOT; fi
if [ -n "${APPLE_BRIDGE_HEAVY_SLOT:-}" ]; then exec "$@"; fi
slots=2
queue=/tmp/apple-bridge-heavy-queue
mkdir -p "$queue"
ticket="$queue/$(date +%s%N)-$$"
touch "$ticket"
held=""
cleanup() { rm -f "$ticket"; [ -n "$held" ] && rm -rf "$held"; return 0; }
trap cleanup EXIT
oldest_live() {
  for t in $(ls "$queue" | sort); do
    pid=${t##*-}
    if kill -0 "$pid" 2>/dev/null; then echo "$queue/$t"; return; fi
    rm -f "$queue/$t"
  done
}
while true; do
  if [ "$(oldest_live)" = "$ticket" ]; then
    for i in $(seq 1 "$slots"); do
      lock="/tmp/apple-bridge-heavy-$i.lock"
      if [ -f "$lock/pid" ] && ! kill -0 "$(cat "$lock/pid")" 2>/dev/null; then rm -rf "$lock"; fi
      if mkdir "$lock" 2>/dev/null; then
        echo $$ > "$lock/pid"
        held="$lock"
        rm -f "$ticket"
        set +e
        APPLE_BRIDGE_HEAVY_SLOT="$i" "$@"
        status=$?
        exit $status
      fi
    done
  fi
  sleep 1
done
