#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
mkdir -p "$root/.build/release-checks"
swiftc -sdk "$SDKROOT" -swift-version 6 -warnings-as-errors -parse-as-library "$root/packaging/release/ReleaseCore.swift" "$root/packaging/release/Checks.swift" -o "$root/.build/release-checks/checks"
"$root/.build/release-checks/checks"
python3 -m unittest discover -s "$root/packaging/release" -p 'test_*.py'
