#!/bin/bash
# Build, notarize, check and publish the npm package. Any failure stops it before npm publish.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
publish=1
case "${1:-}" in
    "") ;;
    --no-publish) publish=0 ;;
    *) echo "Usage: packaging/npm/publish.sh [--no-publish]" >&2; exit 1 ;;
esac
: "${RELEASE_KEYCHAIN:?Set RELEASE_KEYCHAIN}" "${ASC_KEY_PATH:?Set ASC_KEY_PATH}" "${ASC_KEY_ID:?Set ASC_KEY_ID}" "${ASC_ISSUER_ID:?Set ASC_ISSUER_ID}"
cd "$root"
if [[ -n $(git status --porcelain) ]]; then
    echo "Refusing to run: this checkout has uncommitted changes." >&2
    exit 1
fi
if [[ $publish == 1 ]] && ! npm whoami >/dev/null; then
    echo "Refusing to run: log in first with npm login." >&2
    exit 1
fi
mkdir -p dist
out=$(mktemp -d "$root/dist/npm-XXXXXX")
trap 'security lock-keychain "$RELEASE_KEYCHAIN"' EXIT
security unlock-keychain -p "$(<"$HOME/.config/apple-bridge-release/keychain.password")" "$RELEASE_KEYCHAIN"
identity=$(security find-identity -v -p codesigning "$RELEASE_KEYCHAIN" | awk '!found && /Developer ID Application/ {print $2; found=1}')
if [[ -z $identity ]]; then
    echo "Refusing to run: $RELEASE_KEYCHAIN has no Developer ID Application identity." >&2
    exit 1
fi
app="$out/Apple Bridge.app"
APPLE_BRIDGE_SIGNING_GUI=1 packaging/build-app.sh --identity "$identity" "$app"
ditto -c -k --keepParent "$app" "$out/notary.zip"
xcrun notarytool submit "$out/notary.zip" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait --timeout 20m
xcrun stapler staple "$app"
built=$(python3 packaging/npm/build.py --app "$app" --identity "$identity" --output "$out/package")
tarball=$(python3 -c 'import json, sys; print(json.loads(sys.argv[1])["tarball"])' "$built")
python3 packaging/npm/check.py --tarball "$tarball" --app "$app" --identity "$identity" --require-notarized
security lock-keychain "$RELEASE_KEYCHAIN"
trap - EXIT
if [[ $publish == 1 ]]; then
    npm publish "$tarball" --access public
else
    echo "Checked and not published: $tarball"
fi
