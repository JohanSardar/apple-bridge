import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
from signed_app_inputs import source_key

spec = importlib.util.spec_from_file_location("registrations", Path(__file__).with_name("generate-source-registrations.py"))
registrations = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registrations)


class SourceRegistrationsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.package = Path(self.temporary.name)
        native = Path(__file__).resolve().parents[2] / "native"
        shutil.copy(native / "Package.swift", self.package)
        shutil.copytree(native / "Registrations", self.package / "Registrations")
        (self.package / "Sources/AppleSession").mkdir(parents=True)

    def test_new_target_and_own_declaration_register_without_shared_edit(self):
        manifest = self.package / "Package.swift"
        manifest.write_text(manifest.read_text().replace('let sourceTargets: [Target] = [', 'let sourceTargets: [Target] = [\n    .target(name: "ProbeSource", dependencies: ["SourceSupport"]),'))
        declaration = '@MainActor\nextension SessionSources { static func registerProbeSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {} }\n'
        (self.package / "Registrations/ProbeSource.swift").write_text(declaration)
        registrations.generate(self.package)
        output = self.package / "Sources/AppleSession/GeneratedSourceRegistrations.swift"
        text = output.read_text()
        self.assertIn("import ProbeSource\n", text)
        self.assertIn(declaration, text)
        self.assertIn("await registerProbeSource(into: peer, home: home, labEnabled: labEnabled)", text)
        modified = output.stat().st_mtime_ns
        registrations.generate(self.package)
        self.assertEqual(output.stat().st_mtime_ns, modified)

    def test_other_native_packages_are_unchanged(self):
        (self.package / "Package.swift").write_text('let package = Package(name: "AppleTruth")')
        registrations.generate(self.package)
        self.assertFalse((self.package / "Sources/AppleSession/GeneratedSourceRegistrations.swift").exists())

    def test_cache_tracks_declarations_and_generator_only(self):
        root = self.package / "cache-root"
        native = root / "native"
        shutil.copytree(self.package / "Registrations", native / "Registrations")
        shutil.copy(self.package / "Package.swift", native / "Package.swift")
        (native / "Sources/AppleSession").mkdir(parents=True)
        generator = root / "packaging/dev/generate-source-registrations.py"
        generator.parent.mkdir(parents=True)
        generator.write_text("generator-v1")
        with patch("signed_app_inputs.run", return_value="/fixture"):
            before = source_key(root)
            registrations.generate(native)
            self.assertEqual(before, source_key(root))
            declaration = native / "Registrations/NotesSource.swift"
            declaration.write_text(declaration.read_text() + "\n")
            self.assertNotEqual(before, source_key(root))
            before = source_key(root)
            generator.write_text("generator-v2")
            self.assertNotEqual(before, source_key(root))

    def test_missing_declaration_fails(self):
        (self.package / "Registrations/NotesSource.swift").unlink()
        with self.assertRaises(FileNotFoundError):
            registrations.generate(self.package)


if __name__ == "__main__":
    unittest.main()
