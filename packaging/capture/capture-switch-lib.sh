ALL_SOURCES="safari usage notes voice_memos mail phone voicemail calendar contacts reminders messages files mac"
LIVE_HOME="$HOME/Library/Application Support/AppleBridgeLive"

die() { echo "error: $*" >&2; exit 1; }

parse_common() {
  HOME_DIR=""; RESTART=0; DRY=0; HELPER_APP=""; SOURCE_BASE="$HOME"; ARGS=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --home) HOME_DIR="${2:-}"; shift 2 ;;
      --restart-helper) RESTART=1; shift ;;
      --dry-run) DRY=1; shift ;;
      --helper-app) HELPER_APP="${2:-}"; shift 2 ;;
      --source-base) SOURCE_BASE="${2:-}"; shift 2 ;;
      -h|--help) usage; exit 0 ;;
      -*) die "unknown option $1" ;;
      *) ARGS+=("$1"); shift ;;
    esac
  done
  [ -n "$HOME_DIR" ] || die "--home DIR is required (the gateway home; the Helper reads DIR/helper/session/rust-capture.json)"
  [ -d "$HOME_DIR" ] || die "home does not exist: $HOME_DIR"
  HOME_DIR="$(cd "$HOME_DIR" && pwd -P)"
  if [ -d "$HOME_DIR/helper" ]; then
    GATEWAY_HOME="$HOME_DIR"
  elif [ "$(basename "$HOME_DIR")" = helper ] && { [ -d "$(dirname "$HOME_DIR")/run" ] || [ -f "$(dirname "$HOME_DIR")/state.db" ]; }; then
    GATEWAY_HOME="$(dirname "$HOME_DIR")"
  elif [ -f "$HOME_DIR/state.db" ] || [ -f "$HOME_DIR/runtime.json" ] || [ -d "$HOME_DIR/run" ]; then
    GATEWAY_HOME="$HOME_DIR"
    if [ "${DRY:-0}" != 1 ]; then mkdir -p "$GATEWAY_HOME/helper"; fi
  else
    die "$HOME_DIR is not a gateway home (no helper/ directory and no state.db, runtime.json or run/): the Helper would not read a switch file written here"
  fi
  HELPER_HOME="$GATEWAY_HOME/helper"
  live="$(cd "$LIVE_HOME" 2>/dev/null && pwd -P || true)"
  if [ -n "$live" ] && [ "$GATEWAY_HOME" = "$live" ] && [ "${AB_FLIP_LIVE:-}" != "yes" ]; then
    die "$GATEWAY_HOME is the live home; re-run with AB_FLIP_LIVE=yes to confirm"
  fi
  CONFIG="$HELPER_HOME/session/rust-capture.json"
}

verify_visible() {
  local expect_present="$1" pid arg
  if [ "$expect_present" = 1 ]; then
    [ -f "$CONFIG" ] || die "wrote nothing the Helper can see: $CONFIG is missing"
    python3 -c 'import json,sys; c=json.load(open(sys.argv[1])); assert c.get("sources"), "no sources"' "$CONFIG" || die "$CONFIG is not a valid switch file with sources"
  fi
  pid="$(helper_pid || true)"
  if [ -n "$pid" ]; then
    arg="$(ps -ww -o command= -p "$pid" 2>/dev/null | sed -n 's/.* --home \(.*\) --gateway-pid.*/\1/p; s/.* --home \(.*\)$/\1/p' | head -1)"
    if [ -n "$arg" ]; then
      [ "$(cd "$arg" 2>/dev/null && pwd -P)" = "$HELPER_HOME" ] || die "running Helper pid $pid uses home '$arg' but the switch file is under '$HELPER_HOME'"
      echo "ok: running Helper pid $pid reads $CONFIG"
    fi
  else
    echo "ok: switch file is where the next Helper start reads it: $CONFIG"
  fi
}

expand_sources() {
  local spec="$1" out="" s
  if [ "$spec" = "all" ]; then echo "$ALL_SOURCES"; return; fi
  for s in $(echo "$spec" | tr ',' ' '); do
    case " $ALL_SOURCES " in *" $s "*) out="$out $s" ;; *) die "unsupported source '$s' (supported: $ALL_SOURCES)" ;; esac
  done
  echo "$out"
}

source_root() {
  case "$1" in
    safari) echo "$SOURCE_BASE/Library/Safari" ;;
    usage) echo "$SOURCE_BASE/Library/Biome" ;;
    notes) echo "$SOURCE_BASE/Library/Group Containers/group.com.apple.notes" ;;
    messages) echo "$SOURCE_BASE/Library/Messages" ;;
    voice_memos) echo "$SOURCE_BASE/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings" ;;
    mail) echo "$SOURCE_BASE/Library/Mail/V10/MailData" ;;
    phone) echo "$SOURCE_BASE/Library/Application Support/CallHistoryDB" ;;
    voicemail) echo "$SOURCE_BASE/Library/Group Containers/group.com.apple.FaceTime/com.apple.facetimemessagestored/Data Store" ;;
    files) echo "$SOURCE_BASE/Library/Mobile Documents/com~apple~CloudDocs/Bridge Test" ;;
    calendar|contacts|reminders|mac) echo "$HELPER_HOME/session/facts/$1" ;;
  esac
}

root_probe() {
  case "$1" in
    safari) echo "History.db" ;;
    usage) echo "streams" ;;
    notes) echo "NoteStore.sqlite" ;;
    voice_memos) echo "CloudRecordings.db" ;;
    mail) echo "Envelope Index" ;;
    phone) echo "CallHistory.storedata" ;;
    voicemail) echo "FaceTimeMessageStore-local.sqlitedb" ;;
    messages) echo "chat.db" ;;
    *) echo "" ;;
  esac
}

find_session_binary() {
  if [ -n "${AB_SESSION_BINARY:-}" ]; then echo "$AB_SESSION_BINARY"; return; fi
  if [ -n "$HELPER_APP" ]; then echo "$HELPER_APP/Contents/MacOS/ab-session"; return; fi
  echo ""
}

preflight_binary() {
  local binary; binary="$(find_session_binary)"
  if [ -z "$binary" ]; then
    echo "warn: no ab-session binary given (--helper-app APP or AB_SESSION_BINARY); skipping the binary check"
    return 0
  fi
  [ -x "$binary" ] || die "ab-session not found or not executable: $binary"
  grep -aq "session/rust-capture.json" "$binary" || die "$binary was built without the rust-capture feature (it would reject --capture-home)"
  echo "ok: $binary supports the capture switch"
}

atomic_write() {
  local target="$1" tmp
  tmp="$(mktemp "$target.tmp.XXXXXX")"
  cat > "$tmp"
  chmod 600 "$tmp"
  mv "$tmp" "$target"
}

helper_pid() {
  local pid; pid="$(cat "$HELPER_HOME/helper.pid" 2>/dev/null || true)"
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && echo "$pid"
}

helper_env_check() {
  local pid; pid="$(helper_pid || true)"
  if [ -z "$pid" ]; then echo "helper: not running for this home (no live pid in $HELPER_HOME/helper.pid)"; return 0; fi
  if ps -E -ww -p "$pid" 2>/dev/null | tr ' ' '\n' | grep -q '^AB_SESSION_PROXY=1$'; then
    echo "helper pid $pid: AB_SESSION_PROXY=1 (the session proxy is in front of the Helper)"
  else
    echo "helper pid $pid: AB_SESSION_PROXY is NOT 1. The switch cannot take effect until the gateway is started with AB_SESSION_PROXY=1 (and AB_SESSION_BINARY or a bundled Contents/MacOS/ab-session)."
  fi
}

restart_helper() {
  local pid; pid="$(helper_pid || true)"
  [ -n "$pid" ] || { echo "restart: no running Helper for this home; the next Helper start reads the switch"; return 0; }
  echo "restart: SIGTERM Helper pid $pid (the gateway supervisor relaunches it and the new Helper reads the switch)"
  kill -TERM "$pid"
}

show_config() {
  if [ -f "$CONFIG" ]; then python3 -c 'import json,sys; c=json.load(open(sys.argv[1])); print("switch file:", sys.argv[1]); print("rust sources:", ", ".join(sorted(c.get("sources",{}))) or "(none)")' "$CONFIG"; else echo "switch file absent: every source is the verbatim Swift proxy"; fi
}
