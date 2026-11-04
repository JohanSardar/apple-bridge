#!/bin/bash
set -uo pipefail
. "$(dirname "$0")/capture-switch-lib.sh"
usage() {
  cat <<U
usage: unflip-capture.sh <sources|all> --home DIR [--restart-helper] [--dry-run]

Returns the listed sources to the Swift capture. "all" moves the whole switch file aside as
rust-capture.json.off-<timestamp> (absent file = verbatim Swift proxy, and the Swift SessionProxy
stops passing --capture-home). The capture-state directory is kept: a stale record can only cause a rescan.
Nothing is restarted unless --restart-helper is given.
Refuses the live home unless AB_FLIP_LIVE=yes.
U
}
parse_common "$@"
[ ${#ARGS[@]} -eq 1 ] || { usage; exit 2; }
[ -f "$CONFIG" ] || { echo "no switch file at $CONFIG: nothing is flipped"; exit 0; }
if [ "${ARGS[0]}" = all ]; then
  stamp="$(date +%Y%m%d-%H%M%S)"
  echo "moving $CONFIG to $CONFIG.off-$stamp"
  [ "$DRY" = 1 ] && { echo "dry run: nothing changed"; exit 0; }
  mv "$CONFIG" "$CONFIG.off-$stamp"
else
  SOURCES="$(expand_sources "${ARGS[0]}")" || exit 1
  NEW="$(python3 - "$CONFIG" $SOURCES <<'P'
import json, sys
cfg = json.load(open(sys.argv[1]))
for name in sys.argv[2:]:
    cfg.get("sources", {}).pop(name, None)
print(json.dumps(cfg, indent=2, sort_keys=True))
P
)"
  echo "new switch contents:"; echo "$NEW"
  [ "$DRY" = 1 ] && { echo "dry run: nothing changed"; exit 0; }
  cp -p "$CONFIG" "$CONFIG.prev"
  if python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get("sources") else 1)' <(echo "$NEW"); then
    echo "$NEW" | atomic_write "$CONFIG"
  else
    mv "$CONFIG" "$CONFIG.off-$(date +%Y%m%d-%H%M%S)"
  fi
fi
echo
show_config
verify_visible 0
helper_env_check
if [ "$RESTART" = 1 ]; then restart_helper; else echo "not restarted: the rollback takes effect at the next Helper start (re-run with --restart-helper)"; fi
cat <<V

verify:
  ls "$HELPER_HOME/session/"
  ps -ax -o pid,command | grep '[a]b-session'     (no --capture-home for fully unflipped homes)
  the first Swift capture after the restart arrives with the Swift generation (one rescan, never a false deletion)
re-flip: $(dirname "$0")/flip-capture.sh ${ARGS[0]} --home "$HOME_DIR" --restart-helper
V
