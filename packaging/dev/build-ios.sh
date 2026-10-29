#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROFILE="${APPLE_BRIDGE_IOS_PROFILE:-$HOME/.config/apple-bridge/ios/dev.mobileprovision}"
OUTPUT="${APPLE_BRIDGE_IOS_OUTPUT:-$ROOT/.build/ios}"
command -v xcodegen >/dev/null
mkdir -p "$OUTPUT" "$HOME/Library/MobileDevice/Provisioning Profiles"
security cms -D -i "$PROFILE" > "$OUTPUT/profile.plist"
UUID=$(/usr/libexec/PlistBuddy -c 'Print UUID' "$OUTPUT/profile.plist")
NAME=$(/usr/libexec/PlistBuddy -c 'Print Name' "$OUTPUT/profile.plist")
cp "$PROFILE" "$HOME/Library/MobileDevice/Provisioning Profiles/$UUID.mobileprovision"
xcodegen generate --spec "$ROOT/ios/project.yml" --project "$ROOT/ios" >/dev/null
xcodebuild -quiet -project "$ROOT/ios/AppleBridge.xcodeproj" -scheme AppleBridge -configuration Debug -sdk iphoneos -destination generic/platform=iOS -derivedDataPath "$OUTPUT/DerivedData" "PROVISIONING_PROFILE_SPECIFIER=$NAME" build
APP="$OUTPUT/DerivedData/Build/Products/Debug-iphoneos/AppleBridge.app"
codesign --verify --deep --strict "$APP"
mkdir -p "$OUTPUT/staging/Payload"
rm -rf "$OUTPUT/staging/Payload/AppleBridge.app"
ditto "$APP" "$OUTPUT/staging/Payload/AppleBridge.app"
rm -f "$OUTPUT/AppleBridge.ipa"
(cd "$OUTPUT/staging" && /usr/bin/zip -qr "$OUTPUT/AppleBridge.ipa" Payload)
printf '%s\n' "$OUTPUT/AppleBridge.ipa"
