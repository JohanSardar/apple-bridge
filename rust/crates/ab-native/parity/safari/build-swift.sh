#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
out=${1:?output directory required}
support=${2:?compiled SourceSupport directory required}
mkdir -p "$out"
cd "$root"
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
flags=(-sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O)
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -module-name SafariSource -emit-module -emit-object -whole-module-optimization native/Sources/SafariSource/*.swift -emit-module-path "$out/SafariSource.swiftmodule" -o "$out/SafariSource.o"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -I "$out" rust/crates/ab-native/parity/safari/oracle.swift "$out/SafariSource.o" "$support/SourceSupport.o" -lsqlite3 -o "$out/safari-oracle"
