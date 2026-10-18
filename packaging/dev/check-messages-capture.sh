#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
if [[ ! -f native/.build/native-output ]] || [[ ! -x "$(cat native/.build/native-output)/MessagesCaptureChecks" ]]; then
    APPLE_BRIDGE_NATIVE_TARGET=MessagesCaptureChecks packaging/dev/build-native.sh >/dev/null
fi
output=$(cat native/.build/native-output)
"$output/MessagesCaptureChecks"
