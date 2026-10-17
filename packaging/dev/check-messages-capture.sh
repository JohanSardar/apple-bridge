#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
if [[ ! -f native/.build/native-output ]]; then
    packaging/dev/build-native.sh >/dev/null
fi
output=$(cat native/.build/native-output)
"$output/MessagesCaptureChecks"
