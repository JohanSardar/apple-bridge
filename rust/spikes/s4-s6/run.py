import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[3]
subprocess.run(["cargo", "build", "--locked", "--manifest-path", "rust/spikes/s4-s6/Cargo.toml"], cwd=root, check=True)
subprocess.run([sys.executable, "rust/spikes/s4-s6/replay.py"], cwd=root, check=True)
