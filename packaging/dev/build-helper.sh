#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
lock=/tmp/apple-bridge-heavy-build.lock
if ! mkdir "$lock" 2>/dev/null; then
    echo 'Heavy build slot is occupied' >&2
    exit 1
fi
trap 'rmdir "$lock"' EXIT
"$root/packaging/dev/create-identity.sh"
cd "$root/native"
if [[ ${APPLE_BRIDGE_SWIFTC:-1} == 1 ]]; then
    export SDKROOT=${APPLE_BRIDGE_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk}
    output=.build/clt-release
    mkdir -p "$output"
    swiftc -sdk "$SDKROOT" -target arm64-apple-macosx26.0 -swift-version 6 -O -parse-as-library -emit-module -emit-object -whole-module-optimization -module-name MessagesSource Sources/MessagesSource/*.swift -emit-module-path "$output/MessagesSource.swiftmodule" -o "$output/MessagesSource.o"
    swiftc -sdk "$SDKROOT" -target arm64-apple-macosx26.0 -swift-version 6 -O -parse-as-library -emit-module -emit-object -whole-module-optimization -module-name AppleSession -I "$output" Sources/AppleSession/*.swift -emit-module-path "$output/AppleSession.swiftmodule" -o "$output/AppleSession.o"
    swiftc -sdk "$SDKROOT" -target arm64-apple-macosx26.0 -swift-version 6 -O -parse-as-library -I "$output" Sources/AppleBridgeHelper/*.swift "$output/MessagesSource.o" "$output/AppleSession.o" -o "$output/AppleBridgeHelper"
else
    swift build -c release -j 2
    output=.build/release
fi
app="$HOME/Applications/AppleBridgeDev/AppleBridgeHelper.app"
mkdir -p "$app/Contents/MacOS"
cp "$output/AppleBridgeHelper" "$app/Contents/MacOS/AppleBridgeHelper"
cp "$root/packaging/dev/Info.plist" "$app/Contents/Info.plist"
"$root/packaging/dev/sign.sh" "$app"
codesign -d -r- "$app"
