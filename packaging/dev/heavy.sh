#!/bin/bash
set -euo pipefail
if [ -z "${APPLE_BRIDGE_KEEP_SDKROOT:-}" ]; then matching=$("$(dirname "$0")/sdkroot.sh"); [ -n "$matching" ] && export SDKROOT="$matching"; fi
if [ -n "${APPLE_BRIDGE_HEAVY_SLOT:-}" ]; then
  [ "${1:-}" = "--exclusive" ] && shift
  exec "$@"
fi
exclusive=""
if [ "${1:-}" = "--exclusive" ]; then exclusive=1; shift; fi
slots=2
base=${APPLE_BRIDGE_HEAVY_BASE:-/tmp/apple-bridge-heavy}
queue="$base-queue"
mkdir -p "$queue"
ticket="$queue/$(date +%s%N)-$$"
touch "$ticket"
held=""
cleanup() { rm -f "$ticket"; for lock in $held; do rm -rf "$lock"; done; return 0; }
trap cleanup EXIT
oldest_live() {
  for t in $(ls "$queue" | sort); do
    pid=${t##*-}
    if kill -0 "$pid" 2>/dev/null; then echo "$queue/$t"; return; fi
    rm -f "$queue/$t"
  done
}
count() { set -- $held; echo $#; }
while true; do
  if [ "$(oldest_live)" = "$ticket" ]; then
    for i in $(seq 1 "$slots"); do
      lock="$base-$i.lock"
      case " $held " in *" $lock "*) continue ;; esac
      if [ -f "$lock/pid" ] && ! kill -0 "$(cat "$lock/pid")" 2>/dev/null; then rm -rf "$lock"; fi
      if mkdir "$lock" 2>/dev/null; then
        echo $$ > "$lock/pid"
        held="$held $lock"
        if [ -z "$exclusive" ] || [ "$(count)" -eq "$slots" ]; then
          rm -f "$ticket"
          set +e
          label=$i
          [ -n "$exclusive" ] && label=all
          APPLE_BRIDGE_HEAVY_SLOT="$label" "$@"
          exit $?
        fi
      fi
    done
  fi
  sleep 1
done
