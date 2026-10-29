#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IPA="${1:-$ROOT/.build/ios/AppleBridge.ipa}"
scp "$IPA" air:/tmp/AppleBridge.ipa
ssh air '~/.local/bin/pymobiledevice3 apps install /tmp/AppleBridge.ipa'
