#!/bin/bash
set -euo pipefail
if [ -z "${APPLE_BRIDGE_KEEP_SDKROOT:-}" ]; then matching=$("$(dirname "$0")/sdkroot.sh"); [ -n "$matching" ] && export SDKROOT="$matching"; fi
if [ -n "${APPLE_BRIDGE_HEAVY_SLOT:-}" ]; then
  [ "${1:-}" = "--exclusive" ] && shift
  exec "$@"
fi
exclusive=""
if [ "${1:-}" = "--exclusive" ]; then exclusive=1; shift; fi
"$(dirname "$0")/trim-disk.sh" || true
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
usable() {
  if [ -n "$exclusive" ]; then echo "$slots"; return; fi
  memory=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
  if [ -z "${APPLE_BRIDGE_HEAVY_SLOTS:-}" ] && [ "$memory" -gt 0 ] && [ "$memory" -lt 12884901888 ]; then echo 1; return; fi
  if [ -n "${APPLE_BRIDGE_HEAVY_SLOTS:-}" ]; then echo "$APPLE_BRIDGE_HEAVY_SLOTS"; return; fi
  level=$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 100)
  load=$(sysctl -n vm.loadavg 2>/dev/null | awk '{printf "%d", $2}')
  cpus=$(sysctl -n hw.ncpu 2>/dev/null || echo 8)
  if [ "$level" -lt "${APPLE_BRIDGE_HEAVY_MIN_FREE_PCT:-20}" ] || [ "${load:-0}" -gt $((cpus * 2)) ]; then echo 1; return; fi
  before=$(vm_stat 2>/dev/null | awk '/Swapouts/ {gsub(/\./,"",$2); print $2}')
  sleep 2
  after=$(vm_stat 2>/dev/null | awk '/Swapouts/ {gsub(/\./,"",$2); print $2}')
  if [ -n "$before" ] && [ -n "$after" ] && [ $((after - before)) -gt "${APPLE_BRIDGE_HEAVY_MAX_SWAPOUTS:-256}" ]; then echo 1; else echo "$slots"; fi
}
while true; do
  if [ "$(oldest_live)" = "$ticket" ]; then
    for i in $(seq 1 "$(usable)"); do
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
