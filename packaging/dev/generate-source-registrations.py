#!/usr/bin/env python3
from pathlib import Path
import re
import sys


def generate(package):
    manifest = (package / "Package.swift").read_text()
    if "let sourceTargets: [Target] = [" not in manifest:
        return
    blocks = manifest.split("let sourceTargets: [Target] = [", 1)[1].split("\n]", 1)[0]
    targets = re.findall(r'\.target\(\s*name:\s*"([^"]+)"', blocks)
    if not targets or len(targets) != len(set(targets)):
        raise ValueError("sourceTargets must contain unique named target blocks")
    declarations = [(package / "Registrations" / (name + ".swift")).read_text() for name in targets]
    text = "import Foundation\nimport SourceSupport\n"
    text += "\n".join("import " + name for name in targets) + "\n"
    text += "\n".join(declarations)
    text += "\n@MainActor\nextension SessionSources {\n"
    text += "    static func register(into peer: AppleSession, home: URL, labEnabled: Bool) async {\n"
    text += "".join("        await register" + name + "(into: peer, home: home, labEnabled: labEnabled)\n" for name in targets)
    text += "    }\n}\n"
    destination = package / "Sources/AppleSession/GeneratedSourceRegistrations.swift"
    if not destination.exists() or destination.read_text() != text:
        destination.write_text(text)


if __name__ == "__main__":
    generate(Path(sys.argv[1]).resolve())
