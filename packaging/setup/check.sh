#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
stage=$(mktemp -d /tmp/ab-setup-check.XXXXXX)
trap 'rm -rf "$stage"' EXIT
xcrun swiftc -warnings-as-errors "$root/native/Sources/AppleBridgeHelper/SetupState.swift" "$root/native/Sources/AppleBridgeHelper/SetupWindow.swift" "$root/packaging/setup/PermissionFixture.swift" "$root/packaging/setup/Checks.swift" -o "$stage/check"
"$stage/check"
