#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
out=${1:?output directory required}
mkdir -p "$out"
cd "$root"
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
runner=("$HOME/bin/ab-local")
if [[ ${AB_PARITY_LOCAL:-0} == 1 ]]; then runner=(); fi
flags=(-sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O)
"${runner[@]}" swiftc "${flags[@]}" -parse-as-library -emit-module -emit-object -whole-module-optimization -module-name SourceSupport native/Sources/SourceSupport/*.swift -emit-module-path "$out/SourceSupport.swiftmodule" -o "$out/SourceSupport.o"
"${runner[@]}" swiftc "${flags[@]}" -parse-as-library -emit-module -emit-object -whole-module-optimization -module-name Decoders native/Sources/Decoders/*.swift -emit-module-path "$out/Decoders.swiftmodule" -o "$out/Decoders.o"
"${runner[@]}" swiftc "${flags[@]}" -parse-as-library -I "$out" -emit-module -emit-object -whole-module-optimization -module-name UsageSource native/Sources/UsageSource/*.swift -emit-module-path "$out/UsageSource.swiftmodule" -o "$out/UsageSource.o"
"${runner[@]}" swiftc "${flags[@]}" -parse-as-library -I "$out" rust/crates/ab-native/parity/usage/reader.swift "$out/UsageSource.o" "$out/SourceSupport.o" "$out/Decoders.o" -lsqlite3 -o "$out/usage-oracle"
"${runner[@]}" swiftc "${flags[@]}" -I "$out" rust/crates/ab-native/parity/usage/rollup.swift "$out/UsageSource.o" "$out/SourceSupport.o" "$out/Decoders.o" -lsqlite3 -o "$out/rollup-oracle"
