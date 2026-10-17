#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
package=${1:-"$root/native"}
package=$(cd "$package" && pwd)
if [[ -x "$HOME/.swiftly/bin/swift" ]] && "$HOME/.swiftly/bin/swift" package --version >/dev/null 2>&1; then
    target_args=(-j 2)
    if [[ -n ${APPLE_BRIDGE_NATIVE_TARGET:-} ]]; then target_args+=(--product "$APPLE_BRIDGE_NATIVE_TARGET"); fi
    "$HOME/.swiftly/bin/swift" build "${target_args[@]}" --package-path "$package" -c release -Xswiftc -warnings-as-errors
    output="$package/.build/release"
else
    python3 "$root/packaging/dev/build-native.py" "$package"
    output="$package/.build/clt-release"
fi
printf '%s\n' "$output" > "$package/.build/native-output"
