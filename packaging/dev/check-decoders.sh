#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
output=native/.build/decoder-checks
mkdir -p "$output"
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
swiftc -sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O -parse-as-library -emit-module -emit-object -whole-module-optimization -module-name Decoders native/Sources/Decoders/*.swift -emit-module-path "$output/Decoders.swiftmodule" -o "$output/Decoders.o"
if [[ ${1:-} == --capture-fixtures ]]; then
    swiftc -sdk "$sdk" -target arm64-apple-macosx26.0 -I "$output" lab/fixtures/messages/capture.swift "$output/Decoders.o" -o "$output/FixtureCapture"
    "$output/FixtureCapture" "$root/lab/fixtures/messages"
fi
swiftc -sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O -parse-as-library -I "$output" native/Sources/DecoderChecks/*.swift "$output/Decoders.o" -o "$output/DecoderChecks"
"$output/DecoderChecks" "$root/lab/fixtures/messages"
