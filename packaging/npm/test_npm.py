#!/usr/bin/env python3
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build


def write_executable(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)


class Definition(unittest.TestCase):
    def test_the_repository_copy_cannot_be_published_or_run_scripts(self):
        definition = build.package_definition()
        self.assertIs(definition["private"], True)
        self.assertNotIn("scripts", definition)
        self.assertEqual(definition["bin"], {"bridge": "bin/bridge"})
        self.assertEqual((definition["os"], definition["cpu"]), (["darwin"], ["arm64"]))
        self.assertIn(build.APP, definition["files"])

    def test_only_a_release_package_loses_private(self):
        self.assertIs(build.staged_definition("1.2.3", True)["private"], True)
        release = build.staged_definition("1.2.3", False)
        self.assertNotIn("private", release)
        self.assertEqual(release["version"], "1.2.3")

    def test_the_version_comes_from_the_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text('[workspace]\nmembers = []\n[workspace.package]\nedition = "2024"\nversion = "4.5.6"\n[workspace.dependencies]\nserde = { version = "1" }\n')
            self.assertEqual(build.workspace_version(manifest), "4.5.6")
        self.assertRegex(build.workspace_version(), r"^\d+\.\d+\.\d+")


class Tree(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.app = Path(self.directory.name) / build.APP
        for name in build.EXECUTABLES:
            write_executable(self.app / "Contents/MacOS" / name, "code")
        for path in ["Contents/Info.plist", build.GATEWAY_PLIST]:
            (self.app / path).parent.mkdir(parents=True, exist_ok=True)
            (self.app / path).write_text("plist")

    def test_a_flat_app_passes(self):
        self.assertEqual(build.tree_problems(self.app), [])

    def test_anything_a_tarball_would_lose_is_refused(self):
        (self.app / "Contents/link").symlink_to("Info.plist")
        (self.app / "Contents/Empty").mkdir()
        (self.app / "Contents/Frameworks/Sparkle.framework").mkdir(parents=True)
        (self.app / "Contents/Frameworks/Sparkle.framework/Sparkle").write_text("code")
        problems = "\n".join(build.tree_problems(self.app))
        self.assertIn("link is a symlink", problems)
        self.assertIn("Empty is an empty folder", problems)
        self.assertIn("Sparkle.framework is a framework", problems)

    def test_a_missing_piece_is_refused(self):
        (self.app / "Contents/MacOS/ab-session").unlink()
        (self.app / build.GATEWAY_PLIST).unlink()
        problems = "\n".join(build.tree_problems(self.app))
        self.assertIn("Contents/MacOS/ab-session is missing", problems)
        self.assertIn("dev.applebridge.gateway.plist is missing", problems)

    def test_the_trust_requirement_matches_the_helper(self):
        self.assertEqual(build.code_requirement("bridge", "ABCDEF"), 'identifier "bridge" and certificate leaf = H"abcdef"')
        self.assertEqual(build.code_requirement("bridge", build.ADHOC), 'identifier "bridge"')


class Launcher(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        package = self.root / "lib/node_modules/applebridge"
        shutil.copy2(HERE / "bin/bridge", self._parent(package / "bin/bridge"))
        write_executable(package / build.APP / "Contents/MacOS/bridge", '#!/bin/sh\nprintf "%s\\n" "$0" "$@"\n')
        self.command = self.root / "prefix bin/bridge"
        self.command.parent.mkdir()
        self.command.symlink_to(package / "bin/bridge")
        self.stubs = self.root / "stubs"
        self.environment = dict(os.environ)

    @staticmethod
    def _parent(path):
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def run_as(self, system, machine, release, *arguments):
        fake = self.stubs / "fake-uname"
        write_executable(fake, f'#!/bin/sh\ncase "$1" in -s) echo {system};; -m) echo {machine};; -r) echo {release};; esac\n')
        launcher = self.command.resolve()
        launcher.write_text((HERE / "bin/bridge").read_text().replace("/usr/bin/uname", str(fake)))
        return subprocess.run([str(self.command), *arguments], capture_output=True, text=True, env=self.environment)

    def test_linux_gets_a_clear_refusal(self):
        result = self.run_as("Linux", "x86_64", "6.8.0")
        self.assertEqual(result.returncode, 1)
        self.assertIn("needs a Mac with Apple silicon", result.stderr)

    def test_an_intel_mac_gets_a_clear_refusal(self):
        result = self.run_as("Darwin", "x86_64", "25.1.0")
        self.assertEqual(result.returncode, 1)
        self.assertIn("needs a Mac with Apple silicon", result.stderr)

    def test_an_old_mac_gets_a_clear_refusal(self):
        result = self.run_as("Darwin", "arm64", "24.6.0")
        self.assertEqual(result.returncode, 1)
        self.assertIn("needs macOS 26", result.stderr)

    def test_a_supported_mac_runs_the_binary_inside_the_app_through_the_link(self):
        result = self.run_as("Darwin", "arm64", "25.6.0", "stats", "--home", "a b")
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual(lines[1:], ["stats", "--home", "a b"])
        self.assertTrue(lines[0].endswith(f"lib/node_modules/applebridge/{build.APP}/Contents/MacOS/bridge"), lines[0])

    def test_path_cannot_choose_the_binary_it_runs(self):
        other = self.root / "other"
        write_executable(other / build.APP / "Contents/MacOS/bridge", "#!/bin/sh\necho hijacked\n")
        for tool in ["realpath", "dirname"]:
            write_executable(self.stubs / tool, f"#!/bin/sh\necho {other}/bin/bridge\n")
        self.environment["PATH"] = f"{self.stubs}{os.pathsep}{os.environ['PATH']}"
        result = self.run_as("Darwin", "arm64", "25.6.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("hijacked", result.stdout)


class Versions(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.app = Path(self.directory.name) / build.APP
        (self.app / "Contents").mkdir(parents=True)

    def make(self, reported, status=0, marketing="1.2.3", bundle="1.2.3"):
        write_executable(self.app / "Contents/MacOS/bridge", f"#!/bin/sh\necho {reported}\nexit {status}\n")
        (self.app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleShortVersionString": marketing, "CFBundleVersion": bundle}))
        return build.version_problems(self.app, "1.2.3")

    def test_matching_versions_pass(self):
        self.assertEqual(self.make("bridge 1.2.3"), [])

    def test_a_longer_version_is_not_a_match(self):
        self.assertIn("bridge 1.2.30", self.make("bridge 1.2.30")[0])

    def test_a_failing_command_is_not_a_match(self):
        self.assertIn("exit 3", self.make("bridge 1.2.3", status=3)[0])

    def test_the_app_version_must_match(self):
        self.assertEqual(len(self.make("bridge 1.2.3", marketing="9", bundle="1")), 2)


class Publish(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.npm = self.root / "packaging/npm"
        self.npm.mkdir(parents=True)
        shutil.copy2(HERE / "publish.sh", self.npm / "publish.sh")
        self.calls = self.root / "calls"
        self.bin = self.root / "bin"
        write_executable(self.bin / "git", "#!/bin/sh\n")
        write_executable(self.bin / "ditto", "#!/bin/sh\n")
        write_executable(self.bin / "xcrun", "#!/bin/sh\n")
        write_executable(self.bin / "security", f'#!/bin/sh\necho "security $1" >> {self.calls}\n[ "$1" = find-identity ] && echo "  1) ABC123 \\"Developer ID Application: Test\\""\nexit 0\n')
        write_executable(self.bin / "npm", f'#!/bin/sh\necho "npm $*" >> {self.calls}\n')
        write_executable(self.root / "packaging/build-app.sh", "#!/bin/sh\n")
        (self.npm / "build.py").write_text('import json, sys\nprint(json.dumps({"tarball": sys.argv[sys.argv.index("--output") + 1] + "/checked.tgz"}))\n')
        self.set_check(0)
        home = self.root / "home/.config/apple-bridge-release"
        home.mkdir(parents=True)
        (home / "keychain.password").write_text("test")
        self.environment = {**os.environ, "HOME": str(self.root / "home"), "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}", "RELEASE_KEYCHAIN": "k", "ASC_KEY_PATH": "p", "ASC_KEY_ID": "i", "ASC_ISSUER_ID": "s"}

    def set_check(self, status):
        (self.npm / "check.py").write_text(f"import sys\nsys.exit({status})\n")

    def publish(self, *arguments):
        return subprocess.run([str(self.npm / "publish.sh"), *arguments], capture_output=True, text=True, env=self.environment)

    def logged(self):
        return self.calls.read_text().splitlines()

    def test_it_publishes_exactly_the_checked_tarball_with_the_keychain_locked(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.logged()
        self.assertRegex(lines[-1], r"^npm publish .*/dist/npm-[^/]+/package/checked\.tgz --access public$")
        self.assertEqual(lines[-2], "security lock-keychain")

    def test_a_failed_check_never_publishes_and_locks_the_keychain(self):
        self.set_check(1)
        self.assertNotEqual(self.publish().returncode, 0)
        lines = self.logged()
        self.assertFalse([line for line in lines if line.startswith("npm publish")])
        self.assertEqual(lines[-1], "security lock-keychain")

    def test_no_publish_stops_after_the_checks(self):
        result = self.publish("--no-publish")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse([line for line in self.logged() if line.startswith("npm")])

    def test_an_unlogged_in_publish_stops_before_building(self):
        write_executable(self.bin / "npm", "#!/bin/sh\nexit 1\n")
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
