#!/bin/bash
set -euo pipefail
if [ -z "${APPLE_BRIDGE_KEEP_SDKROOT:-}" ]; then matching=$("$(dirname "$0")/sdkroot.sh"); [ -n "$matching" ] && export SDKROOT="$matching"; fi
if [ -n "${APPLE_BRIDGE_HEAVY_SLOT:-}" ]; then
  case "${1:-}" in --exclusive|--light|--sim) shift ;; esac
  exec "$@"
fi
exclusive=""
if [ "${1:-}" = "--exclusive" ]; then exclusive=1; shift; fi
light=""
if [ "${1:-}" = "--light" ]; then light=1; shift; fi
sim=""
if [ "${1:-}" = "--sim" ]; then sim=1; shift; fi
# Commands that may measure time, touch real Apple data or compile Swift. Sim-lane jobs never overlap them.
sensitive() {
  case " $* " in
    *--exclusive*|*real*|*agent*|*quiet*|*atency*|*ATENCY*|*perf*|*Perf*|*PERF*) return 0 ;;
    *swift*|*build-helper*|*build-truth*|*build-native*|*xcodebuild*) return 0 ;;
    *APPLE_BRIDGE_HEAVY_NO_SIM_LANE=*) return 0 ;;
  esac
  return 1
}
# The light lane is for quick checks. Native builds and other sensitive jobs would block it for minutes, so they use the
# normal lane instead.
too_heavy_for_light() {
  case " $* " in
    *"make build"*|*"make native"*|*NATIVE_TARGET=*|*build-native*|*build-helper*|*build-truth*|*swift*|*xcodebuild*) return 0 ;;
  esac
  sensitive "$@"
}
if [ -n "$light" ] && too_heavy_for_light "$@"; then
  echo "heavy.sh: native or sensitive job moved from the light lane to the normal lane" >&2
  light=""
fi
# A plain functional sim-level bridge-lab run (sim is the default level) is eligible for the sim lane.
auto_sim() {
  case " $* " in *bridge-lab*) ;; *) return 1 ;; esac
  sensitive "$@" && return 1
  levels=$(printf ' %s ' "$*" | grep -o -- '--level[ =][A-Za-z]*' | sed 's/--level[ =]//' | sort -u)
  [ -z "$levels" ] || [ "$levels" = sim ]
}
# Scenario files with latency or p95 budgets make a run sensitive even when the command does not name them.
marks=""
for arg in "$@"; do
  case "$arg" in *.yaml|*.yml) [ -f "$arg" ] && grep -q -E 'latency|p95' "$arg" && marks=" latency-gate" ;; esac
done
if [ -z "$exclusive$light$sim$marks" ] && [ -z "${APPLE_BRIDGE_HEAVY_NO_SIM_LANE:-}" ] && auto_sim "$@"; then sim=1; fi
# APPLE_BRIDGE_HEAVY_NO_SIM_LANE also keeps sim jobs from running beside this one.
record="${exclusive:+--exclusive }${APPLE_BRIDGE_HEAVY_NO_SIM_LANE:+quiet }$*$marks"
"$(dirname "$0")/trim-disk.sh" || true
slots=2
base=${APPLE_BRIDGE_HEAVY_BASE:-/tmp/apple-bridge-heavy}
normal="$base"
simlock="$base-sim-1.lock"
lightlock="$base-light-1.lock"
simqueue="$base-sim-queue"
if [ -n "$light" ]; then base="$base-light"; slots=1; fi
queue="$base-queue"
mkdir -p "$queue"
stamp() { if [ -n "${APPLE_BRIDGE_HEAVY_PRIORITY:-}" ]; then echo "0-$(date +%s%N)-$$"; else echo "$(date +%s%N)-$$"; fi; }
ticket="$queue/$(stamp)"
printf '%s\n' "$record" > "$ticket"
simticket=""
if [ -n "$sim" ]; then mkdir -p "$simqueue"; simticket="$simqueue/$(stamp)"; touch "$simticket"; fi
held=""
cleanup() { rm -f "$ticket" ${simticket:+"$simticket"}; for lock in $held; do rm -rf "$lock"; done; return 0; }
trap cleanup EXIT
oldest_live() {
  for t in $(ls "$1" | sort); do
    pid=${t##*-}
    if kill -0 "$pid" 2>/dev/null; then echo "$1/$t"; return; fi
    rm -f "$1/$t"
  done
}
count() { set -- $held; echo $#; }
stale() { [ -f "$1/pid" ] && ! kill -0 "$(cat "$1/pid")" 2>/dev/null; }
usable() {
  if [ -n "$exclusive" ] || [ -n "$light" ]; then echo "$slots"; return; fi
  memory=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
  if [ -z "${APPLE_BRIDGE_HEAVY_SLOTS:-}" ] && [ "$memory" -gt 0 ] && [ "$memory" -lt 12884901888 ]; then
    # Small hosts open a second slot only for non-sensitive jobs beside non-sensitive holders with clear memory headroom.
    if sensitive "$record" || [ ! -d "$base-1.lock" ] || stale "$base-1.lock" || sensitive_holder; then echo 1; return; fi
    level=$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 0)
    if [ "$level" -ge "${APPLE_BRIDGE_SMALL_SECOND_SLOT_FREE_PCT:-40}" ] && [ "$(cpu_idle)" -ge "${APPLE_BRIDGE_SMALL_SECOND_SLOT_IDLE_PCT:-30}" ]; then echo "$slots"; else echo 1; fi
    return
  fi
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
# A holder's recorded command, or its process command line when it predates recording.
holder_text() {
  if [ -f "$1/cmd" ]; then cat "$1/cmd"; return; fi
  [ -f "$1/pid" ] && ps -ww -o command= -p "$(cat "$1/pid")" 2>/dev/null
}
# True while a normal-lane holder is sensitive. A holder whose command is not yet known counts as sensitive.
sensitive_holder() {
  for lock in "$normal"-[0-9]*.lock; do
    [ -d "$lock" ] || continue
    stale "$lock" && continue
    text=$(holder_text "$lock")
    [ -n "$text" ] || return 0
    sensitive "$text" && return 0
  done
  return 1
}
# True when the next normal-lane job is sensitive, so a new sim job would only make it wait.
sensitive_next() {
  next=$(oldest_live "$normal-queue")
  [ -n "$next" ] || return 1
  text=$(cat "$next" 2>/dev/null)
  [ -n "$text" ] || text=$(ps -ww -o command= -p "${next##*-}" 2>/dev/null)
  sensitive "$text"
}
# The sim lane runs beside one normal job only while the Air has clear headroom.
sim_ok() {
  sensitive_holder && return 1
  sensitive_next && return 1
  level=$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 0)
  [ "$level" -ge "${APPLE_BRIDGE_SIM_MIN_FREE_PCT:-45}" ] || return 1
  [ "$(cpu_idle)" -ge "${APPLE_BRIDGE_SIM_MIN_IDLE_PCT:-50}" ] || return 1
  before=$(vm_stat 2>/dev/null | awk '/Swapouts/ {gsub(/\./,"",$2); print $2}')
  sleep 2
  after=$(vm_stat 2>/dev/null | awk '/Swapouts/ {gsub(/\./,"",$2); print $2}')
  [ -n "$before" ] && [ -n "$after" ] && [ $((after - before)) -le "${APPLE_BRIDGE_SIM_MAX_SWAPOUTS:-64}" ]
}
# Heavy jobs wait out real host pressure, at most 20 minutes. macOS load averages count tasks blocked on disk, so they are
# not used. Sensitive jobs hold while CPU idle is below 40%, free memory is below 25% or more than 2,048 pages swap in 2 s;
# other heavy jobs hold only through memory thrash (free memory below 15% or more than 8,192 swapped pages in 2 s).
cpu_idle() {
  top -l 2 -n 0 -s 1 2>/dev/null | awk '/CPU usage/ {idle=$7} END {gsub(/%/,"",idle); printf "%d", (idle==""?100:idle)}'
}
thrashing() {
  level=$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 100)
  [ "$level" -lt "$2" ] && return 0
  if [ "$1" -gt 0 ] && [ "$(cpu_idle)" -lt "$1" ]; then return 0; fi
  before=$(vm_stat 2>/dev/null | awk '/Swapouts|Swapins/ {gsub(/\./,"",$2); total+=$2} END {print total}')
  sleep 2
  after=$(vm_stat 2>/dev/null | awk '/Swapouts|Swapins/ {gsub(/\./,"",$2); total+=$2} END {print total}')
  [ -n "$before" ] && [ -n "$after" ] && [ $((after - before)) -gt "$3" ]
}
wait_thrash() {
  if [ "$1" = sensitive ]; then
    limits="${APPLE_BRIDGE_THRASH_MIN_IDLE_PCT:-40} ${APPLE_BRIDGE_THRASH_MIN_FREE_PCT:-25} ${APPLE_BRIDGE_THRASH_MAX_SWAP_PAGES:-2048}"
  else
    limits="0 ${APPLE_BRIDGE_EXTREME_MIN_FREE_PCT:-15} ${APPLE_BRIDGE_EXTREME_MAX_SWAP_PAGES:-8192}"
  fi
  deadline=$(( $(date +%s) + ${APPLE_BRIDGE_THRASH_WAIT_SECONDS:-1200} ))
  announced=""
  while thrashing $limits; do
    [ -z "$announced" ] && { echo "heavy.sh: host pressure; holding $1 job until it passes" >&2; announced=1; }
    [ "$(date +%s)" -ge "$deadline" ] && { echo "heavy.sh: pressure wait expired; running anyway" >&2; return; }
    sleep 10
  done
}
# True once an exclusive gate holds every normal slot; new light jobs then wait for it. An exclusive job that is still
# waiting for a slot must not block quick checks.
exclusive_active() {
  held_by_exclusive=0
  for lock in "$normal"-[0-9]*.lock; do
    [ -d "$lock" ] || continue
    stale "$lock" && continue
    [ -f "$lock/cmd" ] && case "$(cat "$lock/cmd")" in --exclusive*) held_by_exclusive=$((held_by_exclusive + 1)) ;; esac
  done
  [ "$held_by_exclusive" -ge 2 ]
}
# Exclusive gates also take the light lane, so short checks cannot run beside a performance measurement.
hold_light() {
  until mkdir "$lightlock" 2>/dev/null; do
    if stale "$lightlock"; then rm -rf "$lightlock"; continue; fi
    sleep 1
  done
  echo $$ > "$lightlock/pid"
  printf '%s\n' "--exclusive light hold" > "$lightlock/cmd"
  held="$held $lightlock"
}
# A sensitive job on a small host waits until it is the only normal-lane holder.
wait_normal_alone() {
  while true; do
    alone=1
    for lock in "$normal"-[0-9]*.lock; do
      [ -d "$lock" ] || continue
      case " $held " in *" $lock "*) continue ;; esac
      stale "$lock" && continue
      alone=""
    done
    [ -n "$alone" ] && return
    sleep 2
  done
}
wait_sim_idle() {
  while [ -d "$simlock" ]; do
    if stale "$simlock"; then rm -rf "$simlock"; break; fi
    sleep 2
  done
}
run() {
  label=$1
  shift
  rm -f "$ticket" ${simticket:+"$simticket"}
  set +e
  APPLE_BRIDGE_HEAVY_SLOT="$label" "$@"
  exit $?
}
while true; do
  if [ "$(oldest_live "$queue")" = "$ticket" ] && { [ -z "$light" ] || ! exclusive_active; }; then
    for i in $(seq 1 "$(usable)"); do
      lock="$base-$i.lock"
      case " $held " in *" $lock "*) continue ;; esac
      if stale "$lock"; then rm -rf "$lock"; fi
      if mkdir "$lock" 2>/dev/null; then
        echo $$ > "$lock/pid"
        printf '%s\n' "$record" > "$lock/cmd"
        held="$held $lock"
        if [ -z "$exclusive" ] || [ "$(count)" -eq "$slots" ]; then
          if [ -z "$light" ] && sensitive "$record"; then wait_sim_idle; wait_normal_alone; [ -n "$exclusive" ] && hold_light; wait_thrash sensitive; elif [ -z "$light" ]; then wait_thrash heavy; fi
          label=$i
          [ -n "$exclusive" ] && label=all
          run "$label" "$@"
        fi
      fi
    done
  fi
  if [ -n "$sim" ] && [ "$(oldest_live "$simqueue")" = "$simticket" ]; then
    if stale "$simlock"; then rm -rf "$simlock"; fi
    if mkdir "$simlock" 2>/dev/null; then
      echo $$ > "$simlock/pid"
      printf '%s\n' "$*" > "$simlock/cmd"
      if sim_ok; then held="$held $simlock"; run sim "$@"; fi
      rm -rf "$simlock"
    fi
  fi
  sleep 1
done
