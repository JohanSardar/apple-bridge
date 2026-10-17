#!/bin/bash
set -euo pipefail
keychain="$HOME/Library/Keychains/apple-bridge-dev.keychain-db"
password_file="$HOME/.config/apple-bridge/dev-keychain.pass"
root=$(cd "$(dirname "$0")/../.." && pwd)
[[ $# -ge 1 && $# -le 2 ]] || exit 1
if [[ -n ${2:-} ]]; then
    if [[ -f "$1/Contents/MacOS/bridge" ]]; then
        codesign --force --sign "$2" --options runtime --timestamp "$1/Contents/MacOS/bridge"
    fi
    codesign --force --sign "$2" --options runtime --timestamp --entitlements "$root/packaging/dev/Helper.entitlements" "$1"
    codesign --verify --strict "$1"
    exit
fi
[[ -f "$password_file" ]] || exit 1
[[ $(stat -f %Lp "$password_file") == 600 ]] || exit 1
security unlock-keychain -p "$(cat "$password_file")" "$keychain"
identity=$(security find-certificate -c 'Apple Bridge Dev Signing' -Z "$keychain" | awk '/SHA-1 hash:/{print $3; exit}')
[[ -n "$identity" ]] || exit 1
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$1/Contents/Info.plist")
case "$bundle_id" in
    dev.applebridge.helper|dev.applebridge.truth) ;;
    *) exit 1 ;;
esac
if [[ -f "$1/Contents/MacOS/bridge" ]]; then
    codesign --force --keychain "$keychain" --sign "$identity" --options runtime --timestamp=none "$1/Contents/MacOS/bridge"
fi
codesign --force --keychain "$keychain" --sign "$identity" --options runtime --timestamp=none --entitlements "$root/packaging/dev/Helper.entitlements" --requirements "=designated => identifier \"$bundle_id\" and certificate leaf = H\"$identity\"" "$1"
codesign --verify --strict "$1"
