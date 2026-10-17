#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/lab/truth/native"
directory=$(cat "$root/lab/truth/native/.build/native-output")
output="$directory/apple-truth"
[[ -f "$output" ]] || output="$directory/AppleTruth"
stage=$(mktemp -d "$HOME/Applications/AppleBridgeDev/.truth.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/AppleTruth.app"
mkdir -p "$app/Contents/MacOS"
cp "$output" "$app/Contents/MacOS/apple-truth"
cp "$root/packaging/dev/Truth-Info.plist" "$app/Contents/Info.plist"
"$root/packaging/dev/sign.sh" "$app"
ditto "$app" "$HOME/Applications/AppleBridgeDev/AppleTruth.app"
codesign --verify --strict "$HOME/Applications/AppleBridgeDev/AppleTruth.app"
