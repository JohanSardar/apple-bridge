#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
output=""
if [[ -f native/.build/native-output-1 ]]; then output=$(cat native/.build/native-output-1); fi
if [[ ! -x "$output/MessagesCaptureChecks" ]] || [[ -n $(find native/Sources -name '*.swift' -newer "$output/MessagesCaptureChecks" -print -quit 2>/dev/null) ]]; then
    APPLE_BRIDGE_NATIVE_LAB=1 APPLE_BRIDGE_NATIVE_TARGET=MessagesCaptureChecks packaging/dev/build-native.sh >/dev/null
fi
output=$(cat native/.build/native-output-1)
"$output/MessagesCaptureChecks" "$@"
