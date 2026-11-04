#!/usr/bin/env bash
set -euo pipefail
NODE_VERSION=24.11.0
REPO=JohanSardar/apple-bridge

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
trap 'rm -rf "$work"' EXIT
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

id ab-relay >/dev/null 2>&1 || useradd --system --home /var/lib/ab-relay --shell /usr/sbin/nologin ab-relay
install -d -m 0750 -o root -g ab-relay /etc/ab-relay
install -d -m 0755 /usr/local/lib/ab-relay/bin
install -d -m 0750 -o ab-relay -g ab-relay /var/lib/ab-relay /var/lib/ab-relay-control /var/lib/ab-accounts
install -m 0755 "$source_dir"/bin/* /usr/local/lib/ab-relay/bin/
install -m 0755 "$source_dir/ab-relay-admin" /usr/local/bin/ab-relay-admin
install -m 0644 "$source_dir"/systemd/ab-relay.service /etc/systemd/system/ab-relay.service
touch /etc/ab-relay/owner-keys
chown root:ab-relay /etc/ab-relay/owner-keys
chmod 0640 /etc/ab-relay/owner-keys
if [ -n "$owner_key" ]; then
  /usr/local/bin/ab-relay-admin add-key "$owner_key"
fi
token_file=/etc/ab-relay/admin-token
if [ ! -s "$token_file" ]; then
  (umask 027; head -c 32 /dev/urandom | base64 | tr -d '\n=' | tr '+/' '-_' >"$token_file")
fi
chown root:ab-relay "$token_file"
chmod 0640 "$token_file"
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
{
  printf 'AB_RELAY_ARGS=%s\n' "$args"
  printf 'APPLE_BRIDGE_RELAY_ADMIN=%s\n' "$(cat "$token_file")"
} >/etc/ab-relay/relay.env
chown root:ab-relay /etc/ab-relay/relay.env
chmod 0640 /etc/ab-relay/relay.env

if [ "$accounts" = 1 ]; then
  node_root=/usr/local/lib/ab-relay/node
  if [ "$("$node_root/bin/node" -v 2>/dev/null)" != "v$NODE_VERSION" ]; then
    node_name="node-v$NODE_VERSION-linux-x64"
    curl -fsSL -m 300 -o "$work/$node_name.tar.xz" "https://nodejs.org/dist/v$NODE_VERSION/$node_name.tar.xz"
    curl -fsSL -m 30 "https://nodejs.org/dist/v$NODE_VERSION/SHASUMS256.txt" | grep " $node_name.tar.xz\$" >"$work/node.sha256"
    (cd "$work" && sha256sum -c node.sha256 >/dev/null)
    rm -rf "$node_root"
    install -d "$node_root"
    tar -C "$node_root" --strip-components=1 -xJf "$work/$node_name.tar.xz"
  fi
  install -m 0644 "$source_dir/accounts.mjs" /usr/local/lib/ab-relay/accounts.mjs
  install -m 0644 "$source_dir"/systemd/ab-accounts.service "$source_dir"/systemd/ab-relay-control.service /etc/systemd/system/
  secret=/etc/ab-relay/accounts-secret
  if [ ! -s "$secret" ]; then
    (umask 027; head -c 48 /dev/urandom | base64 | tr -d '\n' >"$secret")
  fi
  chown root:ab-relay "$secret"
  chmod 0640 "$secret"
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
  chown root:ab-relay /etc/ab-relay/accounts.env /etc/ab-relay/control.env
  chmod 0640 /etc/ab-relay/accounts.env /etc/ab-relay/control.env
fi

systemctl daemon-reload
systemctl enable --now ab-relay
systemctl restart ab-relay
if [ "$accounts" = 1 ]; then
  systemctl enable --now ab-accounts ab-relay-control
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
