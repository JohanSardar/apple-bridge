#!/bin/bash
set -uo pipefail
. "$(dirname "$0")/capture-switch-lib.sh"
usage() {
  cat <<U
usage: flip-capture.sh <sources|all> --home DIR [--helper-app APP] [--source-base DIR] [--restart-helper] [--dry-run]

Moves the listed sources from the Swift capture to the Rust capture by writing
DIR/helper/session/rust-capture.json (DIR is the gateway home; the Helper home is DIR/helper) (atomic, previous file kept as rust-capture.json.prev).
Sources: $ALL_SOURCES  (comma separated, or "all")
Nothing is restarted unless --restart-helper is given. The session proxy reads the file when the
Helper starts, so a flip takes effect on the next Helper start.
Refuses the live home unless AB_FLIP_LIVE=yes.
U
}
parse_common "$@"
[ ${#ARGS[@]} -eq 1 ] || { usage; exit 2; }
SOURCES="$(expand_sources "${ARGS[0]}")" || exit 1
preflight_binary
case " $SOURCES " in *" safari "*) echo "note: Safari live tabs stay in the Swift Helper (AppleScript); the Rust reader covers History, Bookmarks, Reading List and iCloud tabs" ;; esac
PLAN="$(mktemp)"
for s in $SOURCES; do
  root="$(source_root "$s")"; probe="$(root_probe "$s")"
  case "$s" in
    calendar|contacts|reminders|mac) mkdir -p "$root" ;;
    *) [ -d "$root" ] || die "$s: root not found: $root"
       [ -z "$probe" ] || [ -e "$root/$probe" ] || die "$s: $probe not found under $root" ;;
  esac
  printf '%s\t%s\n' "$s" "$root" >> "$PLAN"
done
NEW="$(python3 - "$CONFIG" "$PLAN" "$SOURCE_BASE" <<'P'
import json, os, sys
path, plan, base = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    cfg = json.load(open(path))
except FileNotFoundError:
    cfg = {"sources": {}}
cfg.setdefault("sources", {})
cfg.setdefault("watch", True)
cfg.setdefault("poll_ms", 5000)
for line in open(plan):
    name, root = line.rstrip("\n").split("\t", 1)
    entry = {"root": root}
    if name in ("calendar", "contacts", "reminders", "files", "mac", "messages"):
        entry["facts"] = "native"
    if name == "files":
        entry["home"] = base
    cfg["sources"][name] = entry
print(json.dumps(cfg, indent=2, sort_keys=True))
P
)"
rm -f "$PLAN"
echo "new switch contents:"; echo "$NEW"
if [ "$DRY" = 1 ]; then echo "dry run: nothing written (would write $CONFIG)"; exit 0; fi
mkdir -p "$HELPER_HOME/session"
if [ -f "$CONFIG" ]; then cp -p "$CONFIG" "$CONFIG.prev"; fi
echo "$NEW" | atomic_write "$CONFIG"
echo
show_config
verify_visible 1
helper_env_check
if [ "$RESTART" = 1 ]; then restart_helper; else echo "not restarted: the flip takes effect at the next Helper start (re-run with --restart-helper, or restart the gateway/Helper yourself)"; fi
cat <<V

verify:
  python3 -m json.tool "$CONFIG"
  ps -ax -o pid,command | grep '[a]b-session' | grep -- '--capture-home'
  ls "$HELPER_HOME/session/capture-state/"     (one <source>.json per flipped source after its first Rust commit)
  gateway: a Ready frame lists the source and its first Rust capture arrives under the generation the gateway already committed for the source (record ids are kept)
rollback (instant, no data loss):
  $(dirname "$0")/unflip-capture.sh ${ARGS[0]} --home "$HOME_DIR" --restart-helper
V
