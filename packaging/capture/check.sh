#!/bin/bash
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export HOME="$work/home"
mkdir -p "$HOME/Library/Safari" "$HOME/Library/Biome/streams" "$HOME/Library/Application Support/AppleBridgeLive"
touch "$HOME/Library/Safari/History.db" "$HOME/Library/Application Support/AppleBridgeLive/state.db"
gateway="$work/gateway"
mkdir -p "$gateway/helper"
touch "$gateway/state.db"
config="$gateway/helper/session/rust-capture.json"

fail() { echo "capture-switch check failed: $*" >&2; exit 1; }
sources() { python3 -c 'import json,sys; print(",".join(sorted(json.load(open(sys.argv[1]))["sources"])))' "$config"; }

python3 - "$root/rust/crates/ab-session/src/production.rs" "$here/capture-switch-lib.sh" <<'P'
import re, sys
rust = open(sys.argv[1]).read()
supported = set(re.findall(r'"([a-z_]+)"', rust.split("pub const SUPPORTED: &[&str] = &[", 1)[1].split("];", 1)[0]))
shell = open(sys.argv[2]).read()
listed = set(re.search(r'ALL_SOURCES="([^"]+)"', shell).group(1).split())
if listed != supported:
    sys.exit("switch script sources %s differ from ab-session SUPPORTED %s" % (sorted(listed), sorted(supported)))
P

"$here/flip-capture.sh" safari,usage --home "$gateway" --dry-run >/dev/null
[ ! -e "$config" ] || fail "dry run wrote the switch file"

"$here/flip-capture.sh" safari --home "$gateway" >/dev/null
[ "$(sources)" = safari ] || fail "flip wrote $(sources)"
"$here/flip-capture.sh" usage,calendar --home "$gateway" >/dev/null
[ "$(sources)" = calendar,safari,usage ] || fail "second flip did not merge: $(sources)"
[ -f "$config.prev" ] || fail "previous switch file was not kept"
[ "$(python3 -c 'import os,sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777)[2:])' "$config")" = 600 ] || fail "switch file is not mode 600"

"$here/unflip-capture.sh" usage --home "$gateway" >/dev/null
[ "$(sources)" = calendar,safari ] || fail "unflip one left $(sources)"
"$here/unflip-capture.sh" all --home "$gateway" >/dev/null
[ ! -e "$config" ] || fail "unflip all left the switch file"
ls "$gateway/helper/session"/rust-capture.json.off-* >/dev/null 2>&1 || fail "unflip all kept no aside copy"

if "$here/flip-capture.sh" nonsense --home "$gateway" >/dev/null 2>&1; then fail "unknown source accepted"; fi
[ ! -e "$config" ] || fail "unknown source wrote the switch file"

live="$HOME/Library/Application Support/AppleBridgeLive"
if "$here/flip-capture.sh" safari --home "$live" >/dev/null 2>&1; then fail "live home was not refused"; fi
[ ! -e "$live/helper/session/rust-capture.json" ] || fail "refused flip still wrote the live switch"
echo "capture switch checks passed"
