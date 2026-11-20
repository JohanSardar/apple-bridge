import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "packaging/dev/build-bridge.sh"

FAKE_CARGO = """#!/bin/bash
set -euo pipefail
printf '%s\\n' "$@" > "$FAKE_LOG"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --target) target=$2; shift 2 ;;
    --bin) binary=$2; shift 2 ;;
    *) shift ;;
  esac
done
mkdir -p "$CARGO_TARGET_DIR/$target/release"
printf 'built' > "$CARGO_TARGET_DIR/$target/release/$binary"
"""


class BuildBridgeTests(unittest.TestCase):
    def run_script(self, *args):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            cargo = base / "cargo"
            cargo.write_text(FAKE_CARGO)
            cargo.chmod(0o755)
            log = base / "log"
            environment = dict(os.environ, CARGO=str(cargo), CARGO_TARGET_DIR=str(base / "target"), FAKE_LOG=str(log), SDKROOT="/sdk")
            output = base / "out" / "artifact"
            subprocess.run([SCRIPT, output, *args], check=True, env=environment)
            self.assertEqual(output.read_text(), "built")
            self.assertTrue(os.access(output, os.X_OK))
            return log.read_text().splitlines()

    def test_default_builds_the_rust_bridge_for_apple_silicon(self):
        arguments = self.run_script()
        self.assertEqual(arguments[0], "build")
        for flag in ["--locked", "--release"]:
            self.assertIn(flag, arguments)
        self.assertEqual(arguments[arguments.index("-p") + 1], "ab-runtime")
        self.assertEqual(arguments[arguments.index("--bin") + 1], "bridge")
        self.assertEqual(arguments[arguments.index("--target") + 1], "aarch64-apple-darwin")


if __name__ == "__main__":
    unittest.main()
