#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
APP = "Apple Bridge.app"
EXECUTABLES = {"AppleBridgeHelper": "dev.applebridge.helper", "bridge": "bridge", "ab-session": "ab-session"}
GATEWAY_PLIST = "Contents/Library/LaunchAgents/dev.applebridge.gateway.plist"
ADHOC = "-"


class Refused(Exception):
    pass


def run(*command, **options):
    return subprocess.run([str(part) for part in command], capture_output=True, text=True, **options)


def package_definition():
    return json.loads((HERE / "package.json").read_text())


def workspace_version(cargo_manifest=ROOT / "rust/Cargo.toml"):
    match = re.search(r'\[workspace\.package\][^\[]*?^version = "([^"]+)"', cargo_manifest.read_text(), re.MULTILINE)
    if not match:
        raise Refused(f"{cargo_manifest} has no workspace version.")
    return match.group(1)


def relative_files(tree):
    return sorted(str(path.relative_to(tree)) for path in tree.rglob("*") if path.is_file() and not path.is_symlink())


def tree_problems(app):
    problems = []
    for path in sorted(app.rglob("*")):
        name = path.relative_to(app)
        if path.is_symlink():
            problems.append(f"{name} is a symlink, and npm tarballs cannot hold one")
        elif path.is_dir() and not any(path.iterdir()):
            problems.append(f"{name} is an empty folder, and npm tarballs drop them")
        elif path.suffix == ".framework":
            problems.append(f"{name} is a framework, which relies on symlinks")
    for required in [*(f"Contents/MacOS/{name}" for name in EXECUTABLES), GATEWAY_PLIST, "Contents/Info.plist"]:
        if not (app / required).is_file():
            problems.append(f"{required} is missing")
    return problems


def code_paths(app):
    return {name: app if name == "AppleBridgeHelper" else app / "Contents/MacOS" / name for name in EXECUTABLES}


def code_requirement(identifier, identity):
    text = f'identifier "{identifier}"'
    return text if identity == ADHOC else f'{text} and certificate leaf = H"{identity.lower()}"'


def signature_problems(app, identity):
    problems = []
    verified = run("codesign", "--verify", "--deep", "--strict", app)
    if verified.returncode != 0:
        problems.append(f"codesign --verify --deep --strict failed: {verified.stderr.strip()}")
    for name, path in code_paths(app).items():
        requirement = code_requirement(EXECUTABLES[name], identity)
        matched = run("codesign", "--verify", "--strict", f"-R={requirement}", path)
        if matched.returncode != 0:
            problems.append(f"{name} does not satisfy {requirement}")
        details = run("codesign", "-dvv", path).stderr
        if not re.search(r"flags=0x[0-9a-f]*\(([^)]*\b)?runtime\b", details):
            problems.append(f"{name} lacks the hardened runtime")
    return problems


def release_problems(app):
    problems = []
    details = run("codesign", "-dvv", app).stderr
    if "Authority=Developer ID Application:" not in details:
        problems.append("the app is not signed with Developer ID Application")
    stapled = run("xcrun", "stapler", "validate", app)
    if stapled.returncode != 0:
        problems.append("the app has no stapled notarization ticket")
    assessed = run("spctl", "-a", "-vv", "--type", "execute", app)
    if assessed.returncode != 0:
        problems.append("Gatekeeper rejects the app")
    return problems


def staged_definition(version, proof):
    definition = package_definition()
    definition["version"] = version
    if not proof:
        definition.pop("private")
    return definition


def stage(app, output, definition):
    package = output / "package"
    package.mkdir(parents=True)
    copied = run("ditto", app, package / APP)
    if copied.returncode != 0:
        raise Refused(f"Could not copy the app: {copied.stderr.strip()}")
    (package / "bin").mkdir()
    shutil.copy2(HERE / "bin/bridge", package / "bin/bridge")
    (package / "bin/bridge").chmod(0o755)
    (package / "README.md").write_text((HERE / "README.md").read_text().replace("PACKAGE_NAME", definition["name"]))
    shutil.copy2(ROOT / "LICENSE", package / "LICENSE")
    shutil.copy2(ROOT / "THIRD_PARTY_NOTICES.md", package / "THIRD_PARTY_NOTICES.md")
    (package / "package.json").write_text(json.dumps(definition, indent=2) + "\n")
    return package


def pack(package, output):
    packed = run("npm", "pack", "--ignore-scripts", "--json", "--pack-destination", output, package)
    if packed.returncode != 0:
        raise Refused(f"npm pack failed: {packed.stderr.strip()}")
    report = json.loads(packed.stdout)[0]
    expected = set(relative_files(package))
    included = {entry["path"] for entry in report["files"]}
    if included != expected:
        raise Refused(f"npm pack changed the file list. Dropped: {sorted(expected - included)}. Added: {sorted(included - expected)}.")
    return output / report["filename"]


def build(app, identity, output, proof):
    if output.exists():
        raise Refused(f"{output} already exists. Choose a new folder.")
    if identity != ADHOC and not re.fullmatch(r"[0-9A-Fa-f]{40}", identity):
        raise Refused("Pass the signing identity as its SHA-1 fingerprint.")
    if identity == ADHOC and not proof:
        raise Refused("An ad hoc signature can only make a --proof package.")
    problems = tree_problems(app) + signature_problems(app, identity)
    if not proof:
        problems += release_problems(app)
    if problems:
        raise Refused("The app cannot be packaged:\n  " + "\n  ".join(problems))
    version = workspace_version()
    reported = run(app / "Contents/MacOS/bridge", "--version")
    if version not in reported.stdout:
        raise Refused(f"The app's bridge reports {reported.stdout.strip()!r}, but this checkout is version {version}.")
    output.mkdir(parents=True)
    tarball = pack(stage(app, output, staged_definition(version, proof)), output)
    return {
        "name": package_definition()["name"],
        "version": version,
        "publishable": not proof,
        "tarball": str(tarball),
        "bytes": tarball.stat().st_size,
        "sha256": hashlib.sha256(tarball.read_bytes()).hexdigest(),
    }


def main():
    parser = argparse.ArgumentParser(description="Make the npm tarball from an already built and signed Apple Bridge.app. Nothing is published.")
    parser.add_argument("--app", required=True, type=Path, help="The app made by packaging/build-app.sh")
    parser.add_argument("--identity", required=True, help="SHA-1 fingerprint of the certificate the app must be signed with, or - for ad hoc")
    parser.add_argument("--output", required=True, type=Path, help="A new folder for the tarball")
    parser.add_argument("--proof", action="store_true", help="Accept any signature and notarization state, and keep the package private so npm refuses to publish it")
    args = parser.parse_args()
    try:
        print(json.dumps(build(args.app.resolve(), args.identity, args.output.resolve(), args.proof), indent=2))
    except (Refused, OSError, subprocess.SubprocessError) as error:
        print(f"npm build refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
