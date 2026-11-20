#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
sdk=$("$root/packaging/dev/sdkroot.sh")
sdk=${sdk:-$(xcrun --sdk macosx --show-sdk-path)}
stage=$(mktemp -d /tmp/ab-setup-check.XXXXXX)
trap 'rm -rf "$stage"' EXIT
xcrun swiftc -sdk "$sdk" -warnings-as-errors "$root/native/Sources/AppleBridgeHelper/GuideFlow.swift" "$root/packaging/setup/Checks.swift" -o "$stage/check"
"$stage/check"
