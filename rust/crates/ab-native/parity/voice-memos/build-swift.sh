#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
cd "$root"
out=${1:?output directory}
support=${2:?SourceSupport directory}
oracle=rust/crates/ab-native/parity/voice-memos/oracle.swift
mkdir -p "$out"
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
flags=(-sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O)
for module in VoiceMemosSource; do
    "$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -module-name "$module" -emit-module -emit-object -whole-module-optimization native/Sources/"$module"/*.swift -emit-module-path "$out/$module.swiftmodule" -o "$out/$module.o"
done
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -I "$out" "$oracle" "$out/VoiceMemosSource.o" "$support/SourceSupport.o" -lsqlite3 -o "$out/voice-memos-oracle"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -I "$support" -I "$out" rust/crates/ab-native/parity/voice-memos/decoders.swift "$out/VoiceMemosSource.o" "$support/SourceSupport.o" -lsqlite3 -o "$out/decoder-oracle"
