import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_rejects_nonrelease_versions(self):
        for value in ["", "1..2", "1-beta", "-1", "1.2.3.4.5", "18446744073709551616"]:
            with self.assertRaises(ValueError):
                release.version(value)
        self.assertEqual(release.version("1.10.2"), "1.10.2")

    def test_feed_escapes_urls_and_records_archive_length(self):
        with tempfile.TemporaryDirectory(prefix="ab-test-feed-") as temp:
            archive = Path(temp) / "update.zip"
            archive.write_bytes(b"test payload")
            feed = Path(temp) / "appcast.xml"
            release.make_feed(feed, "12", "1.2", "https://example.invalid/update.zip?a=1&b=2", archive, "signature")
            enclosure = ET.parse(feed).find("./channel/item/enclosure")
            self.assertEqual(enclosure.attrib["length"], str(archive.stat().st_size))
            self.assertEqual(enclosure.attrib["url"], "https://example.invalid/update.zip?a=1&b=2")
            self.assertEqual(enclosure.attrib[f"{{{release.NAMESPACE}}}version"], "12")

    def test_rejects_untrusted_download_locations(self):
        for url in ["http://example.invalid/a", "file:///tmp/a", "https://user:password@example.invalid/a", "https:///missing", "https://example.invalid/a#fragment"]:
            with self.assertRaises(ValueError):
                release.https(url)

class TestFlightTests(unittest.TestCase):
    def test_all_profiles_must_be_distribution_profiles_for_exact_bundle(self):
        import sys
        from unittest.mock import patch
        sys.path.insert(0, str(Path(__file__).parent))
        import testflight
        import plistlib
        team = "TESTTEAM01"
        paths = {bundle: bundle for bundle in testflight.PROFILE_SETTINGS}
        def profile(command):
            bundle = command[-1]
            return plistlib.dumps({"TeamIdentifier": [team], "Name": bundle, "UUID": bundle, "Entitlements": {"application-identifier": team + "." + bundle, "aps-environment": "production"}})
        with patch("subprocess.check_output", side_effect=profile):
            settings, decoded = testflight.validate_profiles(paths, team)
            self.assertEqual(len(settings), 5)
            self.assertEqual(len(decoded), 5)
            with self.assertRaises(ValueError):
                testflight.validate_profiles(paths, "OTHERTEAM1")
        development = plistlib.loads(profile([testflight.BUNDLE]))
        development["ProvisionedDevices"] = ["test-device"]
        with patch("subprocess.check_output", return_value=plistlib.dumps(development)):
            with self.assertRaises(ValueError):
                testflight.validate_profiles(paths, team)

    def test_ios_versions_and_push_environment_are_build_settings(self):
        import plistlib
        root = Path(__file__).resolve().parents[2]
        for relative in ["App", "AlarmActivity", "BridgeWidgets", "Extensions/ScreenTimeMonitor", "Extensions/ScreenTimeReport"]:
            info = plistlib.loads((root / "ios" / relative / "Info.plist").read_bytes())
            self.assertEqual(info["CFBundleVersion"], "$(CURRENT_PROJECT_VERSION)")
            self.assertEqual(info["CFBundleShortVersionString"], "$(MARKETING_VERSION)")
        entitlements = plistlib.loads((root / "ios/App/AppleBridge.entitlements").read_bytes())
        self.assertEqual(entitlements["aps-environment"], "$(APPLE_BRIDGE_APNS_ENVIRONMENT)")


if __name__ == "__main__":
    unittest.main()
