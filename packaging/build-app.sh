#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
destination="$root/dist/Apple Bridge.app"
identity=""
app_home=""
api="127.0.0.1:7780"
peer="127.0.0.1:7443"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --notarize) echo "Notarization requires Johan's Apple Developer ID" >&2; exit 1 ;;
        --identity) identity=$2; shift 2 ;;
        --home) app_home=$2; shift 2 ;;
        --api-addr) api=$2; shift 2 ;;
        --peer-addr) peer=$2; shift 2 ;;
        *) destination=$1; shift ;;
    esac
done
[[ ! -e "$destination" ]]
mkdir -p "$(dirname "$destination")"
stage=$(mktemp -d "$(dirname "$destination")/.app-stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/Apple Bridge.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchAgents"
APPLE_BRIDGE_NATIVE_LAB=0 APPLE_BRIDGE_NATIVE_TARGET=AppleBridgeHelper "$root/packaging/dev/build-native.sh" "$root/native"
cp "$(cat "$root/native/.build/native-output-0")/AppleBridgeHelper" "$app/Contents/MacOS/AppleBridgeHelper"
(cd "$root" && go build -p 2 -o "$app/Contents/MacOS/bridge" ./cmd/bridge)
cp "$root/packaging/dev/Info.plist" "$app/Contents/Info.plist"
python3 - "$app" "$app_home" "$api" "$peer" <<'CONFIG'
import plistlib
import sys
from pathlib import Path
app = Path(sys.argv[1])
info = app / "Contents/Info.plist"
values = plistlib.loads(info.read_bytes())
values["CFBundleName"] = "Apple Bridge"
values["CFBundleDisplayName"] = "Apple Bridge"
values["NSContactsUsageDescription"] = "Apple Bridge synchronizes your contacts."
info.write_bytes(plistlib.dumps(values))
arguments = ["bridge", "start", "--role", "gateway", "--api-addr", sys.argv[3], "--peer-addr", sys.argv[4]]
if sys.argv[2]:
    arguments += ["--home", sys.argv[2]]
agent = {"Label": "dev.applebridge.gateway", "BundleProgram": "Contents/MacOS/bridge", "ProgramArguments": arguments, "RunAtLoad": True, "KeepAlive": True, "ThrottleInterval": 2, "LimitLoadToSessionType": "Aqua", "ProcessType": "Background"}
(app / "Contents/Library/LaunchAgents/dev.applebridge.gateway.plist").write_bytes(plistlib.dumps(agent))
CONFIG
"$root/packaging/dev/sign.sh" "$app" "$identity"
ditto "$app" "$destination"
codesign --verify --deep --strict "$destination"
printf '%s\n' "$destination"
