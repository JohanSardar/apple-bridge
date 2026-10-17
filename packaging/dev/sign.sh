#!/bin/bash
set -euo pipefail
keychain="$HOME/Library/Keychains/apple-bridge-dev.keychain-db"
password_file="$HOME/.config/apple-bridge/dev-keychain.pass"
root=$(cd "$(dirname "$0")/../.." && pwd)
[[ $# == 1 && -f "$password_file" ]] || exit 1
[[ $(stat -f %Lp "$password_file") == 600 ]] || exit 1
security unlock-keychain -p "$(cat "$password_file")" "$keychain"
identity=$(security find-certificate -c 'Apple Bridge Dev Signing' -Z "$keychain" | awk '/SHA-1 hash:/{print $3; exit}')
[[ -n "$identity" ]] || exit 1
codesign --force --keychain "$keychain" --sign "$identity" --options runtime --timestamp=none --entitlements "$root/packaging/dev/Helper.entitlements" --requirements "=designated => identifier \"dev.applebridge.helper\" and certificate leaf = H\"$identity\"" "$1"
codesign --verify --strict "$1"
