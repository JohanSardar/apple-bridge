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
    def test_upload_refuses_ipa_without_internal_only_marker(self):
        import sys
        import plistlib
        import zipfile
        sys.path.insert(0, str(Path(__file__).parent))
        import testflight
        with tempfile.TemporaryDirectory(prefix="ab-test-ipa-") as temp:
            ipa = Path(temp) / "AppleBridge.ipa"
            ipa.write_bytes(b"not an IPA")
            with self.assertRaisesRegex(ValueError, "Export Apple Bridge again"):
                testflight.validate_internal_ipa(ipa)
            for marker, bundle, accepted in [(None, testflight.BUNDLE, False), (False, testflight.BUNDLE, False), (True, "other.app", False), (True, testflight.BUNDLE, True)]:
                info = {"CFBundleIdentifier": bundle}
                if marker is not None:
                    info["TFInternalTestingOnly"] = marker
                with zipfile.ZipFile(ipa, "w") as archive:
                    archive.writestr("Payload/AppleBridge.app/Info.plist", plistlib.dumps(info))
                if accepted:
                    testflight.validate_internal_ipa(ipa)
                else:
                    with self.assertRaises(ValueError):
                        testflight.validate_internal_ipa(ipa)

    def test_app_store_icon_is_opaque_and_configured(self):
        import json
        import struct
        root = Path(__file__).resolve().parents[2]
        assets = root / "ios/App/Assets.xcassets/AppIcon.appiconset"
        icon = json.loads((assets / "Contents.json").read_text())["images"][0]
        data = (assets / icon["filename"]).read_bytes()
        self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", data[16:24]), (1024, 1024))
        self.assertEqual(data[25], 2)
        self.assertIn("ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon", (root / "ios/project.yml").read_text())

    def test_ipad_multitasking_and_communication_metadata(self):
        import plistlib
        root = Path(__file__).resolve().parents[2]
        info = plistlib.loads((root / "ios/App/Info.plist").read_bytes())
        self.assertEqual(set(info["UISupportedInterfaceOrientations~ipad"]), {
            "UIInterfaceOrientationPortrait", "UIInterfaceOrientationPortraitUpsideDown",
            "UIInterfaceOrientationLandscapeLeft", "UIInterfaceOrientationLandscapeRight",
        })
        self.assertIn("INSendMessageIntent", info["NSUserActivityTypes"])

    def test_nfc_entitlement_excludes_obsolete_ndef(self):
        import plistlib
        root = Path(__file__).resolve().parents[2]
        entitlements = plistlib.loads((root / "ios/App/AppleBridge.entitlements").read_bytes())
        formats = entitlements["com.apple.developer.nfc.readersession.formats"]
        self.assertNotIn("NDEF", formats)
        self.assertIn("TAG", formats)

    def test_screen_time_report_uses_extensionkit(self):
        import plistlib
        root = Path(__file__).resolve().parents[2]
        info = plistlib.loads((root / "ios/Extensions/ScreenTimeReport/Info.plist").read_bytes())
        self.assertNotIn("NSExtension", info)
        self.assertEqual(info["EXAppExtensionAttributes"]["EXExtensionPointIdentifier"], "com.apple.deviceactivityui.report-extension")
        self.assertIn("ScreenTimeReport:\n    type: extensionkit-extension", (root / "ios/project.yml").read_text())

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
