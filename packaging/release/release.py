#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
SDK = "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk"
SPARKLE_VERSION = "2.10.1"
SPARKLE_SHA256 = "1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27"
NAMESPACE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", NAMESPACE)


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def output(*args):
    return subprocess.check_output([str(a) for a in args], text=True).strip()


def require(name):
    value = os.environ.get(name)
    if not value:
        raise ValueError(f"Set {name} before building a release.")
    return value


def version(value):
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+){0,3}", value) or any(int(v) > 2**64 - 1 for v in value.split('.')):
        raise ValueError("Use a numeric release version, such as 1.0.1.")
    return value


def https(value):
    from urllib.parse import urlsplit
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise ValueError("Use an HTTPS update URL without credentials or fragments.")
    return value


def sparkle():
    tools = ROOT / ".build/release-tools"
    tools.mkdir(parents=True, exist_ok=True)
    archive = tools / "sparkle.tar.xz"
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != SPARKLE_SHA256:
        urllib.request.urlretrieve(f"https://github.com/sparkle-project/Sparkle/releases/download/{SPARKLE_VERSION}/Sparkle-{SPARKLE_VERSION}.tar.xz", archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SPARKLE_SHA256:
        raise ValueError("Sparkle's download failed its checksum. Delete the cached archive and retry.")
    run("tar", "-xf", archive, "-C", tools)
    return tools


def compile_tool(tools):
    target = tools / "release-tool"
    run("swiftc", "-sdk", SDK, "-swift-version", "6", "-warnings-as-errors", "-parse-as-library", ROOT / "packaging/release/ReleaseCore.swift", ROOT / "packaging/release/Tools.swift", "-o", target)
    return target


def make_feed(path, build, marketing, url, payload, signature):
    rss = ET.Element("rss", version="2.0")
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = "Apple Bridge"
    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"Apple Bridge {marketing}"
    ET.SubElement(item, f"{{{NAMESPACE}}}minimumSystemVersion").text = "26.0"
    ET.SubElement(item, "enclosure", {"url": https(url), "length": str(payload.stat().st_size), "type": "application/zip", f"{{{NAMESPACE}}}version": version(build), f"{{{NAMESPACE}}}shortVersionString": version(marketing), f"{{{NAMESPACE}}}edSignature": signature})
    ET.ElementTree(rss).write(path, encoding="utf-8", xml_declaration=True)


def sign_bundle(app, identity, keychain):
    def sign(path, entitlements=None):
        args = ["codesign", "--force", "--sign", identity, "--keychain", str(keychain), "--options", "runtime", "--timestamp", "--preserve-metadata=entitlements"]
        if entitlements:
            args += ["--entitlements", str(entitlements)]
        run(*args, path)
    for path in sorted(app.rglob("*"), key=lambda p: len(p.parts), reverse=True):
        if path.is_symlink():
            continue
        if path.is_file() and "Mach-O" in output("file", "-b", path):
            sign(path, ROOT / "packaging/dev/Helper.entitlements" if path.name == "AppleBridgeHelper" else None)
        elif path.suffix in {".app", ".xpc", ".framework"} and path.is_dir():
            sign(path)
    sign(app, ROOT / "packaging/dev/Helper.entitlements")
    run("codesign", "--verify", "--deep", "--strict", app)
    details = subprocess.run(["codesign", "-dvv", str(app)], capture_output=True, text=True, check=True).stderr
    if "Authority=Developer ID Application:" not in details or "runtime" not in details:
        raise ValueError("The app needs Developer ID Application signing with hardened runtime.")


def notarize(path):
    result = output("xcrun", "notarytool", "submit", path, "--key", require("ASC_KEY_PATH"), "--key-id", require("ASC_KEY_ID"), "--issuer", require("ASC_ISSUER_ID"), "--wait", "--timeout", "20m", "--output-format", "json")
    response = json.loads(result)
    if response.get("status") != "Accepted":
        raise ValueError("Apple did not accept the notarization. Inspect the submission with notarytool log before retrying.")


def build(args):
    os.environ["SDKROOT"] = SDK
    marketing, build_number = version(args.version), version(args.build)
    feed_url, download_url = https(args.feed_url), https(args.download_url)
    destination = Path(args.output).resolve()
    if destination.exists():
        raise ValueError("The release output folder already exists. Choose a new folder.")
    key_file = Path(require("SPARKLE_KEY_FILE")).expanduser().resolve()
    if key_file.is_relative_to(ROOT) and not key_file.is_relative_to(ROOT / ".build"):
        raise ValueError("Keep the update private key outside the repository.")
    key_bytes = base64.b64decode(key_file.read_text().strip(), validate=True)
    if len(key_bytes) != 32 or key_file.stat().st_mode & 0o077:
        raise ValueError("Use a private, mode-600 file containing a base64 Ed25519 seed.")
    identity = os.environ.get("DEVELOPER_ID_APPLICATION", "")
    if not args.prepare_unsigned:
        keychain = Path(require("RELEASE_KEYCHAIN")).expanduser().resolve()
        if not keychain.is_file() or keychain.name == "login.keychain-db":
            raise ValueError("Use the dedicated release keychain prepared by signing.py.")
        identities = output("security", "find-identity", "-v", "-p", "codesigning", keychain)
        if not identity or not re.fullmatch(r"[A-Fa-f0-9]{40}", identity) or not any(identity.upper() in line.upper() and "Developer ID Application:" in line for line in identities.splitlines()):
            raise ValueError("No matching Developer ID Application identity is available. Run signing.py to provision it in the dedicated release keychain before building.")
        for name in ["ASC_KEY_PATH", "ASC_KEY_ID", "ASC_ISSUER_ID"]:
            require(name)
    tools = sparkle()
    tool = compile_tool(tools)
    public_key = output(tool, "public-key", key_file)
    destination.mkdir(parents=True)
    app = destination / "Apple Bridge.app"
    (app / "Contents/MacOS").mkdir(parents=True)
    (app / "Contents/Library/LaunchAgents").mkdir(parents=True)
    run(ROOT / "packaging/dev/build-native.sh", ROOT / "native", env=dict(os.environ, APPLE_BRIDGE_NATIVE_LAB="0", APPLE_BRIDGE_NATIVE_TARGET="AppleBridgeHelper"))
    native_output = Path((ROOT / "native/.build/native-output-0").read_text().strip())
    shutil.copy2(native_output / "AppleBridgeHelper", app / "Contents/MacOS/AppleBridgeHelper")
    run("go", "build", "-p", "2", "-o", app / "Contents/MacOS/bridge", "./cmd/bridge", cwd=ROOT)
    run("ditto", tools / "Sparkle.framework", app / "Contents/Frameworks/Sparkle.framework")
    run("swiftc", "-sdk", SDK, "-swift-version", "6", "-warnings-as-errors", "-parse-as-library", "-F", tools, "-framework", "Sparkle", "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks", ROOT / "packaging/release/ReleaseCore.swift", ROOT / "packaging/release/Launcher.swift", "-o", app / "Contents/MacOS/AppleBridgeLauncher")
    info = plistlib.loads((ROOT / "packaging/dev/Info.plist").read_bytes())
    info.update(CFBundleExecutable="AppleBridgeLauncher", CFBundleName="Apple Bridge", CFBundleDisplayName="Apple Bridge", CFBundleShortVersionString=marketing, CFBundleVersion=build_number, LSUIElement=False, SUFeedURL=feed_url, SUPublicEDKey=public_key, SUVerifyUpdateBeforeExtraction=True, SUEnableAutomaticChecks=True, SUAutomaticallyUpdate=True)
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    agent = dict(Label="dev.applebridge.gateway", BundleProgram="Contents/MacOS/bridge", ProgramArguments=["bridge", "start", "--role", "gateway", "--api-addr", "127.0.0.1:7780", "--peer-addr", "127.0.0.1:7443"], RunAtLoad=True, KeepAlive=True, ThrottleInterval=2, LimitLoadToSessionType="Aqua", ProcessType="Background")
    (app / "Contents/Library/LaunchAgents/dev.applebridge.gateway.plist").write_bytes(plistlib.dumps(agent))
    run("python3", ROOT / "packaging/dev/app-intents.py", app)
    shutil.copy2(tools / "LICENSE", app / "Contents/Resources/Sparkle-LICENSE")
    if args.prepare_unsigned:
        print("R1-BUILD passed=true signed=false")
        return
    sign_bundle(app, identity, keychain)
    submission = destination / "notary.zip"
    run("ditto", "-c", "-k", "--keepParent", app, submission)
    notarize(submission)
    submission.unlink()
    run("xcrun", "stapler", "staple", app)
    run("xcrun", "stapler", "validate", app)
    run("spctl", "-a", "-vv", "--type", "execute", app)
    update = destination / "AppleBridge-update.zip"
    run("ditto", "-c", "-k", "--keepParent", app, update)
    signature = output(tools / "bin/sign_update", "--ed-key-file", key_file, "-p", update)
    feed = destination / "appcast.xml"
    make_feed(feed, build_number, marketing, download_url, update, signature)
    run(tools / "bin/sign_update", "--ed-key-file", key_file, feed)
    run(tools / "bin/sign_update", "--ed-key-file", key_file, "--verify", feed)
    run(tool, "verify", feed, update, public_key)
    with tempfile.TemporaryDirectory(prefix="ab-test-dmg-") as temp:
        stage = Path(temp)
        run("ditto", app, stage / app.name)
        (stage / "Applications").symlink_to("/Applications")
        dmg = destination / "AppleBridge.dmg"
        run("hdiutil", "create", "-volname", "Apple Bridge", "-srcfolder", stage, "-format", "UDZO", dmg)
    run("codesign", "--sign", identity, "--keychain", keychain, "--timestamp", dmg)
    notarize(dmg)
    run("xcrun", "stapler", "staple", dmg)
    run("xcrun", "stapler", "validate", dmg)
    run("spctl", "-a", "-vv", "--type", "open", "--context", "context:primary-signature", dmg)
    evidence = {"signed": True, "notarized": True, "stapled": True, "gatekeeper": True, "update_signature": True, "dmg_sha256": hashlib.sha256(dmg.read_bytes()).hexdigest()}
    (destination / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print("R1-NOTARIZED-DMG passed=true")


def main():
    parser = argparse.ArgumentParser(description="Build a local Apple Bridge release. Nothing is published.")
    parser.add_argument("--version", required=True)
    parser.add_argument("--build", required=True)
    parser.add_argument("--feed-url", required=True)
    parser.add_argument("--download-url", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--prepare-unsigned", action="store_true", help="Compile only; does not produce a distributable DMG")
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Release failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
