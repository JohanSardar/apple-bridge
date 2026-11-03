#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
cd "$root"
out=${1:?output directory}
support=${2:?SourceSupport directory}
decoders=${3:?Decoders directory}
mkdir -p "$out"
out=$(cd "$out" && pwd)
sdk=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
flags=(-sdk "$sdk" -target arm64-apple-macosx26.0 -swift-version 6 -warnings-as-errors -O -D LAB)
python3 - "$out" "$root" <<'PY'
import pathlib,sys
out,root=map(pathlib.Path,sys.argv[1:])
header=root/'native/Sources/MediaExportGuard/include/MediaExportGuard.h'
(out/'MediaExportGuard.modulemap').write_text('module MediaExportGuard { header "'+str(header)+'" export * }\n')
source=(root/'native/Sources/MediaDerivatives/MediaDerivatives.swift').read_text()
original='public static let root = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridge/media-derivatives/v1", isDirectory: true)'
replacement='public static let root = URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("media-derivatives", isDirectory: true)'
assert source.count(original)==1
(out/'MediaDerivatives-fixture.swift').write_text(source.replace(original,replacement))
PY
"$HOME/bin/ab-local" xcrun clang -isysroot "$sdk" -target arm64-apple-macosx26.0 -fobjc-arc -fmodules -O -Wall -Werror -I native/Sources/MediaExportGuard/include -c native/Sources/MediaExportGuard/MediaExportGuard.m -o "$out/MediaExportGuard.o"
media=("$out/MediaDerivatives-fixture.swift")
for file in native/Sources/MediaDerivatives/*.swift; do
    if [[ $file != native/Sources/MediaDerivatives/MediaDerivatives.swift ]]; then media+=("$file"); fi
done
flags+=(-Xcc "-fmodule-map-file=$out/MediaExportGuard.modulemap")
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -module-name MediaDerivatives -emit-module -emit-object -whole-module-optimization "${media[@]}" -emit-module-path "$out/MediaDerivatives.swiftmodule" -o "$out/MediaDerivatives.o"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -I "$decoders" -I "$out" -module-name NotesSource -emit-module -emit-object -whole-module-optimization native/Sources/NotesSource/*.swift -emit-module-path "$out/NotesSource.swiftmodule" -o "$out/NotesSource.o"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -parse-as-library -I "$support" -I "$decoders" -I "$out" rust/crates/ab-native/parity/notes/oracle.swift "$out/NotesSource.o" "$out/MediaDerivatives.o" "$out/MediaExportGuard.o" "$support/SourceSupport.o" "$decoders/Decoders.o" -lsqlite3 -o "$out/notes-oracle"
"$HOME/bin/ab-local" swiftc "${flags[@]}" -I "$support" -I "$decoders" rust/crates/ab-native/parity/notes/plain.swift "$support/SourceSupport.o" "$decoders/Decoders.o" -lsqlite3 -o "$out/plain-oracle"
