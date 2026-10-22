#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
APPLE_BRIDGE_NATIVE_LAB=1 APPLE_BRIDGE_NATIVE_TARGET=RemindersStoreChecks packaging/dev/build-native.sh >/dev/null
output=$(cat native/.build/native-output-1)
"$output/RemindersStoreChecks" "$@"
