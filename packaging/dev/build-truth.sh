#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/lab/truth/native"
directory=$(cat "$root/lab/truth/native/.build/native-output")
output="$directory/apple-truth"
[[ -f "$output" ]] || output="$directory/AppleTruth"
destination=${1:-"$HOME/Applications/AppleBridgeDev/AppleTruth.app"}
mkdir -p "$(dirname "$destination")"
[[ ! -e "$destination" || $# == 0 ]]
stage=$(mktemp -d "$(dirname "$destination")/.stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/AppleTruth.app"
mkdir -p "$app/Contents/MacOS"
cp "$output" "$app/Contents/MacOS/apple-truth"
cp "$root/packaging/dev/Truth-Info.plist" "$app/Contents/Info.plist"
"$root/packaging/dev/sign.sh" "$app"
ditto "$app" "$destination"
codesign --verify --strict "$destination"
