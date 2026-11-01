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

from device import available, wait_available
from activate import activate

SCENARIOS = {"APP-HEALTH-REAL", "APP-HEALTH-WRITE-REAL", "APP-MOTION-REAL", "APP-ALARM-REAL", "APP-DEVICE-REAL", "APP-SCREENTIME-REAL", "APP-LOC-REAL", "APP-CAPTURE-REAL"}
BUNDLE = "com.johansardar.applebridge.ios"


offline_seconds = 0


def active_time():
    return time.monotonic() - offline_seconds


def command(*args, timeout=30):
    global offline_seconds
    while True:
        try:
            result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            result = subprocess.CompletedProcess(args, 124, "", "The iPhone command timed out.")
        if result.returncode and "--device" in args and not available(args[args.index("--device") + 1]):
            device = args[args.index("--device") + 1]
            start = time.monotonic()
            wait_available(device)
            offline_seconds += time.monotonic() - start
            continue
        return result


def launch(*args):
    result = command(*args)
    if result.returncode and "10004" in result.stderr and available(args[args.index("--device") + 1]):
        time.sleep(2)
        result = command(*args)
    if result.returncode == 0:
        try:
            activate(BUNDLE)
        except (OSError, ValueError, RuntimeError) as error:
            return subprocess.CompletedProcess(args, 1, "", str(error))
    return result


def prompt(message):
    message = message.replace("\n", " ").replace("\r", " ")
    path = os.environ.get("AB_IPHONE_AUTOMATION_STEPS")
    if path:
        with pathlib.Path(path).open("a") as log:
            log.write(datetime.datetime.now().strftime("%H:%M ") + message + "\n")
    print("Automation step: " + message, file=sys.stderr, flush=True)


def restore_capture(device, token, temporary):
    launched = launch("xcrun", "devicectl", "device", "process", "launch", "--device", device, "--terminate-existing", BUNDLE, "--ab-real-restore", token)
    if launched.returncode:
        return False
    destination = pathlib.Path(temporary) / "restored.json"
    deadline = active_time() + 30
    while active_time() < deadline:
        destination.unlink(missing_ok=True)
        copied = command("xcrun", "devicectl", "device", "copy", "from", "--device", device, "--domain-type", "appDataContainer", "--domain-identifier", BUNDLE, "--source", "Documents/ab-real-acknowledged.json", "--destination", str(destination))
        if copied.returncode == 0 and destination.exists():
            value = json.loads(destination.read_text())
            if value.get("run") == token and value.get("checks", {}).get("state_restored") and value.get("checks", {}).get("resources_cleaned"):
                return True
        time.sleep(2)
    return False


def run(args):
    if not args.locked:
        return subprocess.call(["lockf", "-k", "-t", "0", "/tmp/ab-iphone.lock", sys.executable, __file__, args.scenario, "--device", args.device, "--locked"])
    wait_available(args.device)
    token = str(uuid.uuid4()).upper()
    with tempfile.TemporaryDirectory(prefix="ab-test-qualification-") as temporary:
        result = pathlib.Path(temporary) / "result.json"
        prompt(f"activate Apple Bridge on the iPhone for {args.scenario}")
        launch_result = pathlib.Path(temporary) / "launch.json"
        prompts = {"APP-HEALTH-REAL": "Health read access", "APP-HEALTH-WRITE-REAL": "Health read and write access", "APP-MOTION-REAL": "Motion & Fitness", "APP-ALARM-REAL": "Alarms", "APP-DEVICE-REAL": "Location for Wi-Fi", "APP-SCREENTIME-REAL": "Screen Time", "APP-LOC-REAL": "Location (Always and Precise)", "APP-CAPTURE-REAL": "Face ID, Camera, then Face ID and Microphone"}
        prompt(f"tap Allow on {prompts[args.scenario]} on the iPhone if prompted (up to 3 minutes)")
        launched = launch("xcrun", "devicectl", "device", "process", "launch", "--device", args.device, "--terminate-existing", "--json-output", str(launch_result), BUNDLE, "--real-qualification", args.scenario, token)
        if launched.returncode:
            print(launched.stderr.strip(), file=sys.stderr, flush=True)
            print(json.dumps({"checks": {"device_launch": False}, "counts": {}, "digests": {}}))
            return 1
        deadline = active_time() + 210
        phase = None
        last_result = None
        while active_time() < deadline:
            result.unlink(missing_ok=True)
            copied = command("xcrun", "devicectl", "device", "copy", "from", "--device", args.device, "--domain-type", "appDataContainer", "--domain-identifier", BUNDLE, "--source", f"Documents/qualification/{token}.json", "--destination", str(result))
            if copied.returncode == 0 and result.exists():
                data = json.loads(result.read_text())
                if data.get("run") != token or data.get("scenario") != args.scenario:
                    time.sleep(2)
                    continue
                last_result = {"checks": dict(data["checks"]), "counts": dict(data["counts"]), "digests": dict(data["digests"])}
                current = data.get("phase")
                if current and current != phase:
                    phase = current
                    deadline = active_time() + 210
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
        safe = last_result or {"checks": {}, "counts": {}, "digests": {}}
        safe["checks"]["result_before_deadline"] = False
        if args.scenario == "APP-CAPTURE-REAL":
            safe["checks"]["state_restored"] = restore_capture(args.device, token, temporary)
        print(json.dumps(safe, sort_keys=True))
        return 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", choices=sorted(SCENARIOS))
    parser.add_argument("--device", default=os.environ.get("APPLE_BRIDGE_REAL_DEVICE", ""))
    parser.add_argument("--locked", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if not arguments.device:
        parser.error("set APPLE_BRIDGE_REAL_DEVICE or --device to the paired physical iPhone identifier")
    return run(arguments)


if __name__ == "__main__":
    sys.exit(main())
