#!/bin/sh
set -eu
metadata=$(mktemp)
trap 'rm -f "$metadata"' EXIT HUP INT TERM
rust/check.sh metadata --all-features --format-version 1 --locked > "$metadata"
python3 rust/lab/check-independence.py < "$metadata"
python3 rust/lab/test-independence.py
