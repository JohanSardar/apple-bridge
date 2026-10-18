#!/usr/bin/env python3
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import time
import uuid

script = Path(__file__).with_name("sign.sh").resolve()
target = str(Path(sys.argv[1]).resolve())
domain = "gui/" + str(os.getuid())
label = "dev.applebridge.sign." + uuid.uuid4().hex
with tempfile.TemporaryDirectory(prefix="apple-bridge-sign-") as directory:
    stage = Path(directory)
    result = stage / "result"
    stdout = stage / "stdout"
    stderr = stage / "stderr"
    job = stage / "agent.plist"
    command = '"$@"; sign_exit=$?; printf "%s\\n" "$sign_exit" > "$0"; exit "$sign_exit"'
    arguments = ["/bin/bash", "-c", command, str(result), str(script), target, *sys.argv[2:]]
    job.write_bytes(plistlib.dumps({
        "Label": label,
        "ProgramArguments": arguments,
        "EnvironmentVariables": {"APPLE_BRIDGE_SIGNING_GUI": "1"},
        "LimitLoadToSessionType": "Aqua",
        "ProcessType": "Interactive",
        "RunAtLoad": True,
        "StandardOutPath": str(stdout),
        "StandardErrorPath": str(stderr),
    }))
    job.chmod(0o600)
    subprocess.run(["launchctl", "bootstrap", domain, str(job)], check=True)
    try:
        deadline = time.monotonic() + 45
        while not result.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        for path, output in [(stdout, sys.stdout), (stderr, sys.stderr)]:
            if path.exists():
                output.write(path.read_text())
        if not result.exists():
            raise SystemExit("GUI-session signing timed out")
        raise SystemExit(int(result.read_text().strip()))
    finally:
        subprocess.run(["launchctl", "bootout", domain + "/" + label], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
