#!/bin/bash
[ "$(uname)" = Darwin ] || exit 0
major=$(sw_vers -productVersion | cut -d. -f1)
developer=$(xcode-select -p 2>/dev/null) || exit 0
for candidate in "$developer/SDKs/MacOSX$major.sdk" "$developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX$major.sdk"; do
  if [ -d "$candidate" ]; then
    cd "$candidate" && pwd -P
    exit 0
  fi
done
