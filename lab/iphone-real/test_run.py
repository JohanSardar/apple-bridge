import json
import contextlib
import io
import types
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import run as runner


class RealRunnerTest(unittest.TestCase):
    def test_reconnect_does_not_consume_the_tap_window(self):
        failed = subprocess.CompletedProcess([], 1)
        succeeded = subprocess.CompletedProcess([], 0)
        with patch.object(runner.subprocess, "run", side_effect=[failed, succeeded]), patch.object(runner, "available", return_value=False), patch.object(runner, "wait_available") as reconnect, patch.object(runner.time, "monotonic", side_effect=[10, 25, 100]), patch.object(runner, "offline_seconds", 0):
            result = runner.command("xcrun", "devicectl", "copy", "--device", "test")
            self.assertEqual(result.returncode, 0)
            reconnect.assert_called_once_with("test")
            self.assertEqual(runner.active_time(), 85)

    def test_timeout_retains_only_fresh_redacted_partial_observations(self):
        def command(*args, **kwargs):
            if "launch" in args:
                pathlib.Path(args[args.index("--json-output") + 1]).write_text(json.dumps({"result": {"process": {"processIdentifier": 1}}}))
            if "copy" in args:
                token = pathlib.Path(args[args.index("--source") + 1]).stem
                pathlib.Path(args[args.index("--destination") + 1]).write_text(json.dumps({"run": token, "scenario": "APP-MOTION-REAL", "final": False, "phase": 1, "checks": {"foreground_active": True}, "counts": {"samples": 0}, "digests": {}, "private": "must-not-export"}))
            return subprocess.CompletedProcess(args, 0)
        with tempfile.TemporaryDirectory() as root:
            output = io.StringIO()
            with patch.dict(os.environ, AB_IPHONE_AUTOMATION_STEPS=str(pathlib.Path(root) / "prompts")), patch.object(runner, "command", side_effect=command), patch.object(runner, "wait_available"), patch.object(runner.time, "monotonic", side_effect=[0, 1, 2, 213]), patch.object(runner.time, "sleep"), patch.object(runner, "offline_seconds", 0), contextlib.redirect_stdout(output):
                self.assertEqual(runner.run(types.SimpleNamespace(locked=True, scenario="APP-MOTION-REAL", device="test")), 1)
            value = json.loads(output.getvalue())
            self.assertEqual(value["checks"], {"foreground_active": True, "result_before_deadline": False})
            self.assertEqual(value["counts"], {"samples": 0})
            self.assertNotIn("must-not-export", output.getvalue())

    def test_launch_retries_only_the_transient_pid_lookup_failure(self):
        failed = subprocess.CompletedProcess([], 1, "", "CoreDevice error 10004")
        succeeded = subprocess.CompletedProcess([], 0, "", "")
        with patch.object(runner, "command", side_effect=[failed, succeeded]) as command, patch.object(runner, "available", return_value=True), patch.object(runner.time, "sleep") as sleep:
            self.assertEqual(runner.launch("xcrun", "devicectl", "--device", "phone").returncode, 0)
            self.assertEqual(command.call_count, 2)
            sleep.assert_called_once_with(2)
        with patch.object(runner, "command", return_value=subprocess.CompletedProcess([], 1, "", "unrelated error")) as command:
            self.assertEqual(runner.launch("xcrun", "devicectl", "--device", "phone").returncode, 1)
            self.assertEqual(command.call_count, 1)

    def test_capture_restore_rejects_stale_acknowledgment(self):
        def command(*args, **kwargs):
            if "copy" in args:
                pathlib.Path(args[args.index("--destination") + 1]).write_text(json.dumps({"run": "stale", "checks": {"state_restored": True, "resources_cleaned": True}}))
            return subprocess.CompletedProcess(args, 0)
        with tempfile.TemporaryDirectory() as root, patch.object(runner, "command", side_effect=command), patch.object(runner, "active_time", side_effect=[0, 1, 31]), patch.object(runner.time, "sleep"):
            self.assertFalse(runner.restore_capture("test", "current", root))

    def test_nonce_bound_final_result_and_redaction(self):
        with tempfile.TemporaryDirectory(prefix="ab-test-host-") as temporary:
            root = pathlib.Path(temporary)
            tool = root / "xcrun"
            tool.write_text('''#!/usr/bin/env python3
import json,pathlib,sys
args=sys.argv[1:]
if "list" in args:
    print("test available (paired)")
if "launch" in args:
    pathlib.Path(args[args.index("--json-output")+1]).write_text(json.dumps({"result":{"process":{"processIdentifier":1}}}))
if "copy" in args:
    token=pathlib.Path(args[args.index("--source")+1]).stem
    destination=pathlib.Path(args[args.index("--destination")+1])
    if destination.exists(): sys.exit(1)
    counter=pathlib.Path(__file__).with_name("counter")
    count=int(counter.read_text())+1 if counter.exists() else 1
    counter.write_text(str(count))
    destination.write_text(json.dumps({"scenario":"APP-MOTION-REAL","run":"stale" if count == 1 else token,"final":count != 2,"checks":{"observed":True},"counts":{"samples":1},"digests":{},"phase":1,"prompt":"tap Face ID\\nthen Camera on the iPhone","private":"must-not-export"}))
''')
            tool.chmod(0o700)
            environment = dict(os.environ, AB_IPHONE_AUTOMATION_STEPS=str(root / "prompts.txt"), PATH=str(root) + os.pathsep + os.environ["PATH"])
            process = subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name("run.py")), "APP-MOTION-REAL", "--device", "test", "--locked"], env=environment, capture_output=True, text=True, timeout=10)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertEqual(json.loads(process.stdout), {"checks": {"observed": True}, "counts": {"samples": 1}, "digests": {}})
            self.assertNotIn("must-not-export", process.stdout)
            self.assertEqual((root / "counter").read_text(), "3")
            prompts = (root / "prompts.txt").read_text().splitlines()
            self.assertEqual(len(prompts), 3)
            self.assertTrue(all(line[2] == ":" and "iPhone" in line for line in prompts))
            self.assertEqual(sum("tap Face ID then Camera" in line for line in prompts), 1)


if __name__ == "__main__":
    unittest.main()
