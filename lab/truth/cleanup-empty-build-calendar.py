import argparse
import json
import os
import pathlib
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--truth-app", required=True)
parser.add_argument("--authorized-earlier-build", action="store_true", required=True)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parent

def proof():
    return json.loads(subprocess.check_output(["python3", str(root / "calendar-cleanup-audit.py"), "--empty-proof"]))

before = proof()
if any(count != 0 for count in before["counts"].values()):
    raise SystemExit("Refused: Bridge Test contains events")
with tempfile.TemporaryDirectory(prefix="empty-build-calendar-", dir="/tmp/ablab") as directory:
    request = pathlib.Path(directory) / "request.json"
    report = pathlib.Path(directory) / "report.json"
    request.write_text(json.dumps({"source": "calendar", "action": "cleanup_empty_build_calendar", "tag": "[ab-test authorized-empty-build-cleanup]", "earlier_build_cleanup_authorized": True, "empty_calendar_proof": before}))
    os.chmod(request, 0o600)
    subprocess.run(["open", "-n", "-W", "-g", "-a", args.truth_app, "--args", "fixture", "--request-file", str(request), "--report", str(report)], check=True)
    result = json.loads(report.read_text())
    after = proof()
    passed = result.get("status") == "verified" and len(after["counts"]) == 0
    print(json.dumps({"before_calendars": len(before["counts"]), "unbounded_events": sum(before["counts"].values()), "after_calendars": len(after["counts"]), "appletruth": result.get("status"), "passed": passed}))
    if not passed:
        raise SystemExit(1)
