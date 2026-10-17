#!/bin/bash
set -euo pipefail
slots=2
while true; do
  for i in $(seq 1 "$slots"); do
    lock="/tmp/apple-bridge-heavy-$i.lock"
    if mkdir "$lock" 2>/dev/null; then
      echo $$ > "$lock/pid"
      trap 'rm -rf "$lock"' EXIT
      "$@"
      exit $?
    fi
    if [ -f "$lock/pid" ] && ! kill -0 "$(cat "$lock/pid")" 2>/dev/null; then
      rm -rf "$lock"
    fi
  done
  sleep 2
done
