import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from signed_app_inputs import cache_directory, source_key


class SignedAppCacheDirectoryTests(unittest.TestCase):
    def test_real_runner_prebuild_and_scenarios_share_a_checkout_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            preparation = {"AB_TICKET": "F7b", "AB_BRANCH": "ship/f7b-reminders"}
            scenario = {**preparation, "AB_IDS": "REM-SECTIONS WEB-REM-PARITY"}
            self.assertEqual(cache_directory(root, preparation), cache_directory(root, scenario))
            self.assertTrue(cache_directory(root, scenario).is_relative_to(root / "bin"))

    def test_other_queued_checkouts_cannot_trim_the_prebuilt_helper(self):
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "first"
            second = Path(directory) / "second"
            runner = {"AB_TICKET": "F7b"}
            first_cache = cache_directory(first, runner)
            second_cache = cache_directory(second, runner)
            self.assertTrue(first_cache.is_relative_to(first / "bin"))
            self.assertTrue(second_cache.is_relative_to(second / "bin"))
            entry = first_cache / ("0" * 64)
            entry.mkdir(parents=True)
            helper = entry / "AppleBridgeHelper.app"
            helper.mkdir()
            for index in range(4):
                (second_cache / (str(index) * 64)).mkdir(parents=True)
            scripts = second / "packaging/dev"
            scripts.mkdir(parents=True)
            for name in ["signed-app-cache.py", "signed_app_inputs.py"]:
                shutil.copyfile(Path(__file__).parent / name, scripts / name)
            environment = {key: value for key, value in os.environ.items() if key != "APPLE_BRIDGE_SIGNED_APP_CACHE"}
            environment["AB_TICKET"] = "another"
            subprocess.run([sys.executable, str(scripts / "signed-app-cache.py"), "--trim"], env=environment, check=True, capture_output=True)
            self.assertTrue(helper.exists())
            self.assertEqual(len(list(second_cache.glob("[0-9]" * 64))), 3)

    def test_ordinary_builds_keep_the_shared_cache(self):
        self.assertEqual(cache_directory(Path("/checkout"), {}), Path.home() / "Library/Caches/AppleBridgeDev/signed-apps")

    def test_explicit_cache_directory_takes_precedence(self):
        self.assertEqual(cache_directory(Path("/checkout"), {"AB_TICKET": "F7b", "APPLE_BRIDGE_SIGNED_APP_CACHE": "/configured"}), Path("/configured"))


class AdHocSigningTests(unittest.TestCase):
    def test_lab_cache_never_reads_the_signing_keychain(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, {"APPLE_BRIDGE_SIGNING_IDENTITY": "-", "APPLE_BRIDGE_NATIVE_LAB": "1"}):
                with patch("signed_app_inputs.run", return_value="/fixture") as run:
                    source_key(Path(directory))
                    self.assertFalse(any(call.args[0] == "security" for call in run.call_args_list))

    def test_ad_hoc_signing_is_lab_only_and_never_launches_a_gui_job(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "commands"
            for name in ["security", "launchctl"]:
                command = root / name
                command.write_text("#!/bin/sh\nexit 99\n")
                command.chmod(0o700)
            command = root / "codesign"
            command.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$SIGN_LOG"\n')
            command.chmod(0o700)
            environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"], SIGN_LOG=str(log), APPLE_BRIDGE_SIGNING_IDENTITY="-", APPLE_BRIDGE_NATIVE_LAB="1")
            script = Path(__file__).with_name("sign.sh")
            result = subprocess.run(["/bin/bash", str(script), str(root / "Fixture.app")], env=environment, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(log.read_text().splitlines()), 2)
            self.assertIn("--sign -", log.read_text())
            log.unlink()
            environment["APPLE_BRIDGE_NATIVE_LAB"] = "0"
            result = subprocess.run(["/bin/bash", str(script), str(root / "Fixture.app")], env=environment, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(log.exists())


if __name__ == "__main__":
    unittest.main()
