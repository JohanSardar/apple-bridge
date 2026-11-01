import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("device", pathlib.Path(__file__).with_name("device.py"))
device = importlib.util.module_from_spec(spec)
spec.loader.exec_module(device)


class DeviceAvailabilityTest(unittest.TestCase):
    def test_unavailable_is_not_available(self):
        for state, expected in [("available (paired)", True), ("unavailable (paired)", False), ("available (unpaired)", False)]:
            result = type("Result", (), {"returncode": 0, "stdout": "phone id " + state})()
            with patch.object(device.subprocess, "run", return_value=result):
                self.assertEqual(device.available("id"), expected)
                self.assertFalse(device.available("other"))

    def test_active_tunnel_requires_paired_identity(self):
        for pairing, expected in [("paired", True), ("unpaired", False)]:
            def discover(args, **kwargs):
                pathlib.Path(args[args.index("--json-output") + 1]).write_text(json.dumps({"result": {"devices": [{"identifier": "id", "connectionProperties": {"pairingState": pairing, "tunnelState": "connected"}}]}}))
                return type("Result", (), {"returncode": 0, "stdout": "phone id connected"})()
            with patch.object(device.subprocess, "run", side_effect=discover):
                self.assertEqual(device.available("id"), expected)
                self.assertFalse(device.available("other"))

    def test_discovery_timeout_is_treated_as_disconnect(self):
        with patch.object(device.subprocess, "run", side_effect=device.subprocess.TimeoutExpired("xcrun", 30)):
            self.assertFalse(device.available("id"))

    def test_disconnect_retries_without_requesting_manual_steps(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / "prompts"
            with patch.dict(os.environ, AB_JOHAN_PROMPT_FILE=str(path)), patch.object(device, "available", side_effect=[False, False, True]), patch.object(device.time, "sleep") as sleep:
                device.wait_available("id")
            self.assertEqual(sleep.call_args_list, [unittest.mock.call(15), unittest.mock.call(15)])
            self.assertFalse(path.exists())

    def test_disconnect_keeps_retrying_beyond_ten_minutes(self):
        with patch.object(device, "available", side_effect=[False] * 42 + [True]), patch.object(device.time, "sleep") as sleep:
            device.wait_available("id")
        self.assertEqual(sleep.call_count, 42)
        self.assertTrue(all(call == unittest.mock.call(15) for call in sleep.call_args_list))


if __name__ == "__main__":
    unittest.main()
