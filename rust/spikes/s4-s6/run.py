import os
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[3]
subprocess.run(["cargo", "build", "--locked", "--manifest-path", "rust/spikes/s4-s6/Cargo.toml"], cwd=root, check=True)
environment = os.environ.copy()
environment["AB_SPIKE_BINARY"] = str(root / "rust/spikes/s4-s6/target/debug/ab-spike-s4-s6")
subprocess.run(["go", "test", "./lab/scenarios", "./internal/frontdoor", "-run", "TestRust(WebAuthn|MCP)", "-count=1", "-v"], cwd=root, env=environment, check=True)
subprocess.run([sys.executable, "rust/spikes/s4-s6/replay.py"], cwd=root, check=True)
