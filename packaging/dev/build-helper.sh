#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/native"
output=$(cat "$root/native/.build/native-output")
[[ -f "$output/AppleBridgeHelper" ]]
stage=$(mktemp -d "$HOME/Applications/AppleBridgeDev/.helper.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/AppleBridgeHelper.app"
mkdir -p "$app/Contents/MacOS"
cp "$output/AppleBridgeHelper" "$app/Contents/MacOS/AppleBridgeHelper"
cp "$root/packaging/dev/Info.plist" "$app/Contents/Info.plist"
"$root/packaging/dev/sign.sh" "$app"
ditto "$app" "$HOME/Applications/AppleBridgeDev/AppleBridgeHelper.app"
codesign --verify --strict "$HOME/Applications/AppleBridgeDev/AppleBridgeHelper.app"
