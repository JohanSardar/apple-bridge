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

    def test_release_bundles_the_session_binary_beside_the_helper(self):
        with tempfile.TemporaryDirectory(prefix="ab-test-bundle-") as temp:
            app = Path(temp) / "Apple Bridge.app"
            (app / "Contents/MacOS").mkdir(parents=True)
            binary = Path(temp) / "ab-session"
            binary.write_bytes(b"session")
            binary.chmod(0o755)
            bundled = release.bundle_session(app, {"AB_SESSION_BINARY": str(binary)})
            self.assertEqual(bundled, app / "Contents/MacOS/ab-session")
            self.assertEqual(bundled.read_bytes(), b"session")
            self.assertTrue(bundled.stat().st_mode & 0o111)

    def test_release_refuses_a_missing_session_binary(self):
        with tempfile.TemporaryDirectory(prefix="ab-test-bundle-") as temp:
            app = Path(temp) / "Apple Bridge.app"
            (app / "Contents/MacOS").mkdir(parents=True)
            with self.assertRaises(ValueError):
                release.bundle_session(app, {"AB_SESSION_BINARY": str(Path(temp) / "missing")})
            plain = Path(temp) / "plain"
            plain.write_bytes(b"x")
            plain.chmod(0o644)
            with self.assertRaises(ValueError):
                release.bundle_session(app, {"AB_SESSION_BINARY": str(plain)})
            self.assertFalse((app / "Contents/MacOS/ab-session").exists())


if __name__ == "__main__":
    unittest.main()
