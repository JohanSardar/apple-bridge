#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
APPLE_BRIDGE_NATIVE_LAB=1 "$root/packaging/dev/build-native.sh" "$root/native"
output=$(cat "$root/native/.build/native-output-1")
"$output/CalendarChecks"
