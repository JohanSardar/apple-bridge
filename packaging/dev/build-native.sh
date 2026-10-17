#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
package=${1:-"$root/native"}
if [[ -x "$HOME/.swiftly/bin/swift" ]] && "$HOME/.swiftly/bin/swift" package --version >/dev/null 2>&1; then
    "$HOME/.swiftly/bin/swift" build --package-path "$package" -c release -j 2 -Xswiftc -warnings-as-errors
else
    python3 "$root/packaging/dev/build-native.py" "$package"
fi
