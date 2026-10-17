#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/native"
output=$(cat "$root/native/.build/native-output")
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
