#!/usr/bin/env bash
set -euo pipefail
NODE_VERSION=24.11.0
NODE_SHA256=46da9a098973ab7ba4fca76945581ecb2eaf468de347173897044382f10e0a0a
REPO=JohanSardar/apple-bridge

fetch_node_archive() {
  local destination=$1
  curl -fsSL -m 300 -o "$destination" "https://nodejs.org/dist/v$NODE_VERSION/node-v$NODE_VERSION-linux-x64.tar.xz"
  if [ "$(sha256sum "$destination" | cut -d ' ' -f 1)" != "$NODE_SHA256" ]; then
    echo "The Node.js download does not match the pinned checksum. Nothing was installed." >&2
    return 1
  fi
}

[ "${AB_INSTALL_SOURCE_ONLY:-}" != 1 ] || return 0

domain='' public_ip='' owner_key='' accounts=0 bundle='' version=latest max_accounts=25 dns=authoritative dns_project='' dns_zone=''
usage() {
  cat <<'USAGE'
install.sh --domain bridge.example.com [options]

  --domain NAME        The zone this relay owns. Macs get addresses under it.
  --owner-key KEY      Allow your Mac straight away (from: bridge remote key).
  --accounts           Add accounts and invites, for other people. Sign-in is at app.<domain>.
  --public-ip IP       This server's public IPv4 address. Found automatically if left out.
  --bundle FILE        Install from this file instead of downloading a release.
  --version TAG        Release to download. Default: the latest relay release.
  --max-accounts N     Accounts allowed with --accounts. Default: 25.
  --cloud-dns P/Z      Use Google Cloud DNS (project/zone) instead of answering DNS itself.
USAGE
}
while [ $# -gt 0 ]; do
  case "$1" in
    --domain) domain=$2; shift 2 ;;
    --owner-key) owner_key=$2; shift 2 ;;
    --accounts) accounts=1; shift ;;
    --public-ip) public_ip=$2; shift 2 ;;
    --bundle) bundle=$2; shift 2 ;;
    --version) version=$2; shift 2 ;;
    --max-accounts) max_accounts=$2; shift 2 ;;
    --cloud-dns) dns=cloud; dns_project=${2%%/*}; dns_zone=${2##*/}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[ "$(id -u)" = 0 ] || { echo "Run this as root." >&2; exit 1; }
[ -n "$domain" ] || { usage >&2; exit 2; }
[[ "$domain" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$ ]] || { echo "--domain must be a lowercase DNS name." >&2; exit 2; }
[ -n "$owner_key" ] || [ "$accounts" = 1 ] || { echo "Say who can use this relay: --owner-key KEY (from: bridge remote key) or --accounts." >&2; exit 2; }
[ "$(uname -m)" = x86_64 ] || { echo "Only x86_64 servers are supported for now." >&2; exit 1; }
if [ -z "$public_ip" ]; then
  public_ip=$(curl -fsS -4 -m 10 https://api.ipify.org)
fi
[[ "$public_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Could not tell this server's public IP. Pass --public-ip." >&2; exit 1; }

work=$(mktemp -d)
trap 'status=$?; rm -rf "$work"; [ "$status" = 0 ] || echo "The install stopped before it finished. Fix the error above and run it again; repeating it is safe." >&2' EXIT
if [ -z "$bundle" ]; then
  if [ "$version" = latest ]; then
    version=$(curl -fsS -m 20 "https://api.github.com/repos/$REPO/releases?per_page=30" | python3 -c 'import json,sys
print(next(r["tag_name"] for r in json.load(sys.stdin) if r["tag_name"].startswith("relay-")))')
  fi
  name="ab-relay-${version#relay-}-linux-amd64.tar.gz"
  curl -fsSL -m 300 -o "$work/$name" "https://github.com/$REPO/releases/download/$version/$name"
  curl -fsSL -m 30 -o "$work/$name.sha256" "https://github.com/$REPO/releases/download/$version/$name.sha256"
  (cd "$work" && sha256sum -c "$name.sha256" >/dev/null)
  bundle=$work/$name
fi
tar -C "$work" -xzf "$bundle"
source_dir=$(find "$work" -maxdepth 1 -type d -name 'ab-relay-*' | head -1)

service_user() {
  id "$1" >/dev/null 2>&1 || useradd --system --user-group --home "$2" --shell /usr/sbin/nologin "$1"
}
own_state() {
  install -d -m 0750 -o "$1" -g "$1" "$2"
  chown -R "$1:$1" "$2"
}
service_user ab-relay /var/lib/ab-relay
if [ "$accounts" = 1 ]; then
  service_user ab-control /var/lib/ab-relay-control
  service_user ab-accounts /var/lib/ab-accounts
fi
install -d -m 0751 -o root -g root /etc/ab-relay
install -d -m 0755 /usr/local/lib/ab-relay/bin
install -m 0755 "$source_dir"/bin/* /usr/local/lib/ab-relay/bin/
touch /etc/ab-relay/owner-keys
chown root:ab-relay /etc/ab-relay/owner-keys
chmod 0640 /etc/ab-relay/owner-keys
token_file=/etc/ab-relay/admin-token
if [ ! -s "$token_file" ]; then
  (umask 077; head -c 32 /dev/urandom | base64 | tr -d '\n=' | tr '+/' '-_' >"$token_file")
fi
if [ "$dns" = authoritative ]; then
  bind_ip=$(ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p')
  dns_args="-dns-listen $bind_ip:53 -public-ip $public_ip"
else
  dns_args="-dns-project $dns_project -dns-zone $dns_zone"
fi
args="-owner-keys-file /etc/ab-relay/owner-keys -addr :443 -control-host $domain -zone $domain -state /var/lib/ab-relay/state.json -control-acme $dns_args"
if [ "$accounts" = 1 ]; then
  args="$args -app-host app.$domain -app-upstream 127.0.0.1:8789"
fi
(
  umask 077
  {
    printf 'AB_RELAY_ARGS=%s\n' "$args"
    printf 'APPLE_BRIDGE_RELAY_ADMIN=%s\n' "$(cat "$token_file")"
  } >/etc/ab-relay/relay.env
)
chown root:root /etc/ab-relay/relay.env
chmod 0600 /etc/ab-relay/relay.env

if [ "$accounts" = 1 ]; then
  node_root=/usr/local/lib/ab-relay/node
  if [ "$("$node_root/bin/node" -v 2>/dev/null)" != "v$NODE_VERSION" ]; then
    fetch_node_archive "$work/node.tar.xz"
    rm -rf "$node_root"
    install -d "$node_root"
    tar -C "$node_root" --strip-components=1 -xJf "$work/node.tar.xz"
  fi
  install -m 0644 "$source_dir/accounts.mjs" /usr/local/lib/ab-relay/accounts.mjs
  secret=/etc/ab-relay/accounts-secret
  if [ ! -s "$secret" ]; then
    (umask 077; head -c 48 /dev/urandom | base64 | tr -d '\n' >"$secret")
  fi
  cat >/etc/ab-relay/accounts.env <<ENV
AB_PUBLIC_URL=https://app.$domain
AB_ACCOUNTS_DB=/var/lib/ab-accounts/accounts.sqlite
AB_ACCOUNTS_SECRET_FILE=$secret
AB_ACCOUNTS_LISTEN=127.0.0.1:8790
AB_MAX_ACCOUNTS=$max_accounts
ENV
  cat >/etc/ab-relay/control.env <<ENV
AB_PUBLIC_URL=https://app.$domain
AB_RELAY_CONTROL_DB=/var/lib/ab-relay-control/control.sqlite
AB_RELAY_CONTROL_FENCE=/var/lib/ab-relay-control/epoch
AB_RELAY_CONTROL_LISTEN=127.0.0.1:8789
AB_ACCOUNTS_URL=http://127.0.0.1:8790
AB_RELAY_URL=https://$domain
AB_RELAY_CONNECT=127.0.0.1:443
AB_RELAY_ADMIN_TOKEN_FILE=$token_file
ENV
  chown root:root /etc/ab-relay/accounts.env /etc/ab-relay/control.env
  chmod 0600 /etc/ab-relay/accounts.env /etc/ab-relay/control.env
fi

if [ "$accounts" = 1 ]; then
  systemctl stop ab-relay-control ab-accounts 2>/dev/null || true
  chown root:ab-control "$token_file"
  chmod 0640 "$token_file"
  chown root:ab-accounts "$secret"
  chmod 0640 "$secret"
  own_state ab-control /var/lib/ab-relay-control
  own_state ab-accounts /var/lib/ab-accounts
  install -m 0644 "$source_dir"/systemd/ab-accounts.service "$source_dir"/systemd/ab-relay-control.service /etc/systemd/system/
else
  chown root:root "$token_file"
  chmod 0600 "$token_file"
fi
own_state ab-relay /var/lib/ab-relay
install -m 0755 "$source_dir/ab-relay-admin" /usr/local/bin/ab-relay-admin
install -m 0644 "$source_dir"/systemd/ab-relay.service /etc/systemd/system/ab-relay.service
if [ -n "$owner_key" ]; then
  /usr/local/bin/ab-relay-admin add-key "$owner_key"
fi
systemctl daemon-reload
systemctl enable ab-relay
systemctl restart ab-relay
if [ "$accounts" = 1 ]; then
  systemctl enable ab-accounts ab-relay-control
  systemctl restart ab-accounts ab-relay-control
fi
cat <<DONE

Relay installed.

  DNS: if you have not yet, add these two records where $domain's parent zone is hosted:
    ns1.$domain   A   $public_ip
    $domain       NS  ns1.$domain
  Firewall: allow TCP 443, and TCP and UDP 53.

  On your Mac:  bridge remote on --relay-url https://$domain
DONE
if [ "$accounts" = 1 ]; then
  cat <<DONE

  Invite someone:  ab-relay-admin invite --note "their name"
DONE
fi
