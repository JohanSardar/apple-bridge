#!/bin/bash
# Starts a scratch gateway on a temporary home and runs the independent OAuth clients against it.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$repo/rust/target}
(cd "$repo/rust" && cargo build -q -p ab-runtime --bin bridge)
bridge=$target/debug/bridge
home=$(mktemp -d)
port=$((17000 + RANDOM % 2000))
approve=$(mktemp)
cat >"$approve" <<APPROVE
#!/bin/bash
exec "$bridge" connector allow --request "\$1" --code "\$2" --home "$home" >/dev/null
APPROVE
chmod +x "$approve"
"$bridge" start --role gateway --home "$home" --api-addr "127.0.0.1:$port" >"$home/gateway.log" 2>&1 &
gateway=$!
trap 'kill "$gateway" 2>/dev/null || true; rm -rf "$home" "$approve"' EXIT
for _ in $(seq 1 100); do
  curl -fsS -m 1 "http://127.0.0.1:$port/v1/health" >/dev/null 2>&1 && break
  sleep 0.2
done
cd "$here"
[ -d node_modules ] || npm install --silent --no-audit --no-fund
node oauth-clients.mjs "http://127.0.0.1:$port" "$approve"
