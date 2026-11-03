#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../.." && pwd)
out=${1:?output directory required}
mkdir -p "$out"
cd "$root"
"$HOME/bin/ab-local" swiftc -swift-version 5 -emit-library -emit-module -module-name Decoders native/Sources/Decoders/*.swift -o "$out/libDecoders.dylib" -emit-module-path "$out/Decoders.swiftmodule"
"$HOME/bin/ab-local" swiftc -swift-version 5 -parse-as-library -I "$out" -L "$out" -lDecoders -Xlinker -rpath -Xlinker "$out" native/Sources/MessagesSource/MessagesReader.swift native/Sources/MessagesSource/MessageActionWindow.swift rust/crates/ab-native/parity/messages.swift -o "$out/messages-oracle"
