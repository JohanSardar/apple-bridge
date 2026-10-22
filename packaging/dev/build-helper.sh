#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
if [[ $# == 1 && ${APPLE_BRIDGE_SIGNED_CACHE_BUILD:-} != 1 ]]; then
    [[ ! -e "$1" && ! -L "$1" ]]
    if python3 "$root/packaging/dev/signed-app-cache.py" helper "$1"; then exit 0; fi
    rm -rf -- "$1"
fi
APPLE_BRIDGE_NATIVE_TARGET=AppleBridgeHelper "$root/packaging/dev/build-native.sh" "$root/native"
output=$(cat "$root/native/.build/native-output-${APPLE_BRIDGE_NATIVE_LAB:-1}")
[[ -f "$output/AppleBridgeHelper" ]]
destination=${1:-"$HOME/Applications/AppleBridgeDev/AppleBridgeHelper.app"}
mkdir -p "$(dirname "$destination")"
[[ ! -e "$destination" || $# == 0 ]]
stage=$(mktemp -d "$(dirname "$destination")/.stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/AppleBridgeHelper.app"
mkdir -p "$app/Contents/MacOS"
cp "$output/AppleBridgeHelper" "$app/Contents/MacOS/AppleBridgeHelper"
cp "$root/packaging/dev/Info.plist" "$app/Contents/Info.plist"
"$root/packaging/dev/sign.sh" "$app"
ditto "$app" "$destination"
codesign --verify --strict "$destination"
