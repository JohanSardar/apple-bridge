import argparse
import datetime
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import uuid

SCENARIOS = {"APP-HEALTH-REAL", "APP-HEALTH-WRITE-REAL", "APP-MOTION-REAL", "APP-ALARM-REAL", "APP-DEVICE-REAL", "APP-SCREENTIME-REAL", "APP-LOC-REAL", "APP-CAPTURE-REAL"}
BUNDLE = "com.johansardar.applebridge.ios"


def command(*args, timeout=30):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def prompt(message):
    path = pathlib.Path(os.environ.get("AB_JOHAN_PROMPT_FILE", pathlib.Path.home() / "I1-NEEDS-JOHAN.txt"))
    with path.open("a") as log:
        log.write(datetime.datetime.now().strftime("%H:%M ") + message + "\n")
    print(">>> JOHAN: " + message, file=sys.stderr, flush=True)


def run(args):
    if not args.locked:
        return subprocess.call(["lockf", "-k", "-t", "0", "/tmp/ab-iphone.lock", sys.executable, __file__, args.scenario, "--device", args.device, "--locked"])
    token = str(uuid.uuid4()).upper()
    with tempfile.TemporaryDirectory(prefix="ab-test-qualification-") as temporary:
        result = pathlib.Path(temporary) / "result.json"
        prompt(f"unlock the iPhone and keep Apple Bridge open for {args.scenario}")
        launch_result = pathlib.Path(temporary) / "launch.json"
        prompts = {"APP-HEALTH-REAL": "Health read access", "APP-HEALTH-WRITE-REAL": "Health read and write access", "APP-MOTION-REAL": "Motion & Fitness", "APP-ALARM-REAL": "Alarms", "APP-DEVICE-REAL": "Location for Wi-Fi", "APP-SCREENTIME-REAL": "Screen Time", "APP-LOC-REAL": "Location (Always and Precise)", "APP-CAPTURE-REAL": "Face ID, Camera, then Face ID and Microphone"}
        prompt(f"tap Allow on {prompts[args.scenario]} on the iPhone if prompted (up to 3 minutes)")
        launched = command("xcrun", "devicectl", "device", "process", "launch", "--device", args.device, "--terminate-existing", "--json-output", str(launch_result), BUNDLE, "--real-qualification", args.scenario, token)
        if launched.returncode:
            print(launched.stderr.strip(), file=sys.stderr, flush=True)
            print(json.dumps({"checks": {"device_launch": False}, "counts": {}, "digests": {}}))
            return 1
        deadline = time.monotonic() + 210
        phase = None
        while time.monotonic() < deadline:
            result.unlink(missing_ok=True)
            copied = command("xcrun", "devicectl", "device", "copy", "from", "--device", args.device, "--domain-type", "appDataContainer", "--domain-identifier", BUNDLE, "--source", f"Documents/qualification/{token}.json", "--destination", str(result))
            if copied.returncode == 0 and result.exists():
                data = json.loads(result.read_text())
                if data.get("run") != token or data.get("scenario") != args.scenario:
                    time.sleep(2)
                    continue
                current = data.get("phase")
                if current and current != phase:
                    phase = current
                    deadline = time.monotonic() + 210
                    instruction = data.get("prompt")
                    if isinstance(instruction, str):
                        prompt(instruction.replace("\n", " ").replace("\r", " "))
                if data.get("final") is True:
                    safe = {"checks": data["checks"], "counts": data["counts"], "digests": data["digests"]}
                    print(json.dumps(safe, sort_keys=True))
                    return 0
            time.sleep(2)
        launch_data = json.loads(launch_result.read_text())
        pid = launch_data.get("result", {}).get("process", {}).get("processIdentifier")
        if pid is not None:
            command("xcrun", "devicectl", "device", "process", "terminate", "--device", args.device, "--pid", str(pid))
        print(json.dumps({"checks": {"result_before_deadline": False}, "counts": {}, "digests": {}}))
        return 1


parser = argparse.ArgumentParser()
parser.add_argument("scenario", choices=sorted(SCENARIOS))
parser.add_argument("--device", default=os.environ.get("APPLE_BRIDGE_REAL_DEVICE", ""))
parser.add_argument("--locked", action="store_true", help=argparse.SUPPRESS)
arguments = parser.parse_args()
if not arguments.device:
    parser.error("set APPLE_BRIDGE_REAL_DEVICE or --device to the paired physical iPhone identifier")
sys.exit(run(arguments))
