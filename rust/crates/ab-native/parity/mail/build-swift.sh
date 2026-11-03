#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
cd "$root"
out=${1:?output directory}
support=${2:?SourceSupport directory}
mkdir -p "$out"
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
flags=(-sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O)
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -module-name MailSource -emit-module -emit-object -whole-module-optimization native/Sources/MailSource/*.swift -emit-module-path "$out/MailSource.swiftmodule" -o "$out/MailSource.o"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -I "$out" rust/crates/ab-native/parity/mail/oracle.swift "$out/MailSource.o" "$support/SourceSupport.o" -lsqlite3 -o "$out/mail-oracle"
