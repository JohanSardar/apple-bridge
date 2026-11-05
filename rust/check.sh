#!/bin/sh
set -eu
cd "$(dirname "$0")"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
if [ "$(uname -s)" = Darwin ]; then
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/ab-cargo-target}"
    if [ -x "$HOME/bin/ab-local" ]; then
        exec "$HOME/bin/ab-local" cargo "$@"
    fi
fi
exec cargo "$@"
