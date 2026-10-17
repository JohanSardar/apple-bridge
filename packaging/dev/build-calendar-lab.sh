#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/native"
"$root/packaging/dev/build-native.sh" "$root/lab/truth/native"
helper_output=$(cat "$root/native/.build/native-output")
truth_output=$(cat "$root/lab/truth/native/.build/native-output")
apps="$root/native/.build/calendar-apps"
mkdir -p "$apps"
for kind in helper truth; do
    if [ "$kind" = helper ]; then
        name=AppleBridgeHelper
        binary="$helper_output/AppleBridgeHelper"
        executable=AppleBridgeHelper
        plist=Info.plist
    else
        name=AppleTruth
        binary="$truth_output/apple-truth"
        [ -f "$binary" ] || binary="$truth_output/AppleTruth"
        executable=apple-truth
        plist=Truth-Info.plist
    fi
    app="$apps/$name.app"
    mkdir -p "$app/Contents/MacOS"
    cp "$binary" "$app/Contents/MacOS/$executable"
    cp "$root/packaging/dev/$plist" "$app/Contents/Info.plist"
    "$root/packaging/dev/sign.sh" "$app"
done
"$helper_output/CalendarChecks"
