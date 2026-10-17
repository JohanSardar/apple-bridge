#!/bin/bash
set -euo pipefail
umask 077
keychain="$HOME/Library/Keychains/apple-bridge-dev.keychain-db"
password_file="$HOME/.config/apple-bridge/dev-keychain.pass"
identity='Apple Bridge Dev Signing'
ensure_search_path() {
    keychains=()
    found=0
    while IFS= read -r entry; do
        keychains+=("$entry")
        if [[ "$entry" == "$keychain" ]]; then found=1; fi
    done < <(security list-keychains -d user | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//')
    if [[ $found == 0 ]]; then
        security list-keychains -d user -s "${keychains[@]}" "$keychain"
    fi
}
mkdir -p "$(dirname "$password_file")"
if [[ -e "$keychain" ]]; then
    [[ -f "$password_file" ]] || exit 1
    if security find-certificate -c "$identity" "$keychain" >/dev/null 2>&1; then
        ensure_search_path
        exit 0
    fi
fi
if [[ ! -e "$password_file" ]]; then
    openssl rand -hex 32 > "$password_file"
fi
chmod 600 "$password_file"
password=$(cat "$password_file")
work=$(mktemp -d /tmp/apple-bridge-sign.XXXXXX)
trap 'rm -rf "$work"' EXIT
cat > "$work/certificate.cnf" <<'CONFIG'
[req]
distinguished_name = name
x509_extensions = signing
prompt = no
[name]
CN = Apple Bridge Dev Signing
[signing]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
CONFIG
openssl req -new -newkey rsa:2048 -nodes -x509 -days 3650 -config "$work/certificate.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12" -passout "file:$password_file" -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
if [[ ! -e "$keychain" ]]; then
    security create-keychain -p "$password" "$keychain"
fi
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$work/identity.p12" -k "$keychain" -P "$password" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
ensure_search_path
