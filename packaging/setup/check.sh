#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
sdk=$("$root/packaging/dev/sdkroot.sh")
[ -z "$sdk" ] || export SDKROOT=$sdk
stage=$(mktemp -d /tmp/ab-setup-check.XXXXXX)
trap 'rm -rf "$stage"' EXIT
xcrun swiftc -warnings-as-errors "$root/native/Sources/AppleBridgeHelper/SetupState.swift" "$root/packaging/setup/Checks.swift" -o "$stage/check"
"$stage/check"
