#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
"$root/packaging/dev/build-native.sh" "$root/native"
output=$(cat "$root/native/.build/native-output")
"$output/CalendarChecks"
