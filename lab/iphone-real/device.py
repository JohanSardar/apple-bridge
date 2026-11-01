import json
import tempfile
import pathlib
import subprocess
import sys
import time


def available(device):
    try:
        with tempfile.TemporaryDirectory(prefix="ab-test-device-") as temporary:
            output = pathlib.Path(temporary) / "devices.json"
            result = subprocess.run(["xcrun", "devicectl", "list", "devices", "--json-output", str(output)], capture_output=True, text=True, timeout=30)
            if result.returncode:
                return False
            if any(device in row and "unavailable" in row for row in result.stdout.splitlines()):
                return False
            if any(device in row and "available (paired)" in row and "unavailable" not in row for row in result.stdout.splitlines()):
                return True
            if not output.exists():
                return False
            devices = json.loads(output.read_text()).get("result", {}).get("devices", [])
            return any(value.get("identifier") == device and value.get("connectionProperties", {}).get("pairingState") == "paired" and value.get("connectionProperties", {}).get("tunnelState") == "connected" for value in devices)
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return False


def wait_available(device):
    announced = False
    while not available(device):
        if not announced:
            print("Waiting for the paired physical iPhone; retrying every 15 seconds.", file=sys.stderr, flush=True)
            announced = True
        time.sleep(15)


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[2] == "--check":
        sys.exit(0 if available(sys.argv[1]) else 1)
    wait_available(sys.argv[1])
