#!/bin/bash
set -euo pipefail
keychain="$HOME/Library/Keychains/apple-bridge-dev.keychain-db"
root=$(cd "$(dirname "$0")/../.." && pwd)
[[ $# -ge 1 && $# -le 2 ]] || exit 1
if [[ ${APPLE_BRIDGE_SIGNING_GUI:-} != 1 ]]; then
    exec python3 "$root/packaging/dev/sign-gui.py" "$@"
fi
profile="$HOME/.config/apple-bridge/signing/services-dev.provisionprofile"
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$1/Contents/Info.plist")
if [[ $# == 1 && $bundle_id == dev.applebridge.helper && -f $profile && ${APPLE_BRIDGE_SIGNING_IDENTITY:-services} != development ]]; then
    entitlements=$(mktemp)
    trap 'rm -f "$entitlements"' EXIT
    identity=$(python3 - "$profile" "$1" "$root/packaging/dev/Helper.entitlements" "$entitlements" <<'PYPROFILE'
import hashlib
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
profile, app, base, output = map(Path, sys.argv[1:])
data = plistlib.loads(subprocess.check_output(["security", "cms", "-D", "-i", str(profile)]))
grants = data["Entitlements"]
entitlements = plistlib.loads(base.read_bytes())
for key in ["com.apple.application-identifier", "com.apple.developer.team-identifier", "com.apple.developer.weatherkit"]:
    entitlements[key] = grants[key]
if grants["com.apple.developer.weatherkit"] is not True:
    raise SystemExit("services profile does not enable WeatherKit")
output.write_bytes(plistlib.dumps(entitlements))
info = app / "Contents/Info.plist"
metadata = plistlib.loads(info.read_bytes())
metadata["CFBundleIdentifier"] = grants["com.apple.application-identifier"].split(".", 1)[1]
info.write_bytes(plistlib.dumps(metadata))
shutil.copyfile(profile, app / "Contents/embedded.provisionprofile")
print(hashlib.sha1(data["DeveloperCertificates"][0]).hexdigest().upper())
PYPROFILE
)
    codesign --force --sign "$identity" --options runtime --timestamp=none --entitlements "$entitlements" "$1"
    codesign --verify --strict "$1"
    exit
fi
if [[ -n ${2:-} ]]; then
    if [[ -f "$1/Contents/MacOS/bridge" ]]; then
        codesign --force --sign "$2" --options runtime --timestamp "$1/Contents/MacOS/bridge"
    fi
    codesign --force --sign "$2" --options runtime --timestamp --entitlements "$root/packaging/dev/Helper.entitlements" "$1"
    codesign --verify --strict "$1"
    exit
fi
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
