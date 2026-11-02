#!/bin/sh
set -eu
cd "$(dirname "$0")"
export CARGO_BUILD_JOBS=2
if [ "$(uname -s)" = Darwin ]; then
    export CARGO_TARGET_DIR="$HOME/ab-cargo-target"
    exec "$HOME/bin/ab-local" cargo "$@"
fi
exec cargo "$@"
