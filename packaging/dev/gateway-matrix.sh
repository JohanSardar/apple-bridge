#!/usr/bin/env bash
set -euo pipefail
if [[ $(uname -s) != Linux ]]; then
  echo 'gateway-matrix requires Linux; sync this checkout and run make gateway-matrix on ssh build' >&2
  exit 1
fi
: "${SIM_OUTPUT:?}"
exec "$(dirname "$0")/rust-lab.sh" sim
