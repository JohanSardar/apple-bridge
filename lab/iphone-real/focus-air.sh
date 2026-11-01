set -eu
app=${1:-"$HOME/Applications/AppleBridgeDev/AppleBridgeHelper.app"}
report=$(mktemp "$HOME/Library/Application Support/AppleBridgeDev/focus-phone-XXXXXXXX")
trap 'rm -f "$report"' EXIT
codesign --verify --strict -R '=identifier "com.johansardar.applebridge.services"' "$app" >&2
open -n -W -g -a "$app" --args focus-real-checks FOC-STATUS-REAL --report "$report"
if ! /usr/bin/plutil -extract grant_passed raw "$report" | /usr/bin/grep -qx true; then
    cat "$report"
    exit 0
fi
if [ -n "${2:-}" ]; then
    rm -f "$report"
    open -n -W -g -a "$app" --args focus-phone-pack "$2" --report "$report"
    if ! /usr/bin/plutil -extract pack_import_passed raw "$report" | /usr/bin/grep -qx true; then
        cat "$report"
        exit 0
    fi
fi
if [ "$(shortcuts list | /usr/bin/grep -xc 'AB Focus')" -ne 1 ]; then
    printf '{"pack_passed":false}\n'
    exit 0
fi
rm -f "$report"
open -n -W -g -a "$app" --args focus-real-checks FOC-PHONE-SYNC --report "$report"
cat "$report"
