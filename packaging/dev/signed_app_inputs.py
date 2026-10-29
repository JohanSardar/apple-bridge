import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

def require(condition, message):
    if not condition:
        raise RuntimeError(message)

def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT).strip()

def digest_bundle(app):
    digest = hashlib.sha256()
    for path in sorted(app.rglob("*")):
        digest.update(str(path.relative_to(app)).encode() + b"\0")
        digest.update(str(path.lstat().st_mode).encode() + b"\0")
        if path.is_symlink():
            digest.update(os.readlink(path).encode())
        elif path.is_file():
            digest.update(path.read_bytes())
    return digest.hexdigest()

def source_key(root):
    digest = hashlib.sha256()
    wanted = {"Info.plist", "Truth-Info.plist", "Helper.entitlements", "sign.sh", "sign-gui.py", "build-helper.sh", "build-truth.sh", "build-native.sh", "build-native.py", "generate-source-registrations.py", "sdkroot.sh", "signed-app-cache.py", "signed_app_inputs.py", "app-intents.py"}
    for base in [root / "native", root / "lab/truth/native", root / "packaging/dev", root / "packaging/app-intents"]:
        paths = []
        for directory, children, files in os.walk(base):
            children[:] = [name for name in children if name not in {".build", "node_modules", ".git", "__pycache__"}]
            paths.extend(Path(directory) / name for name in files)
        for path in sorted(paths):
            if path == root / "native/Sources/AppleSession/GeneratedSourceRegistrations.swift":
                continue
            if base.name == "dev" and path.name not in wanted:
                continue
            if base.name not in {"dev", "app-intents"} and path.suffix not in {".swift", ".m", ".mm", ".c", ".h", ".modulemap"}:
                continue
            digest.update(str(path.relative_to(root)).encode() + b"\0" + path.read_bytes() + b"\0")
    profile = Path.home() / ".config/apple-bridge/signing/services-dev.provisionprofile"
    if profile.exists():
        digest.update(profile.read_bytes())
    lab = (os.environ.get("APPLE_BRIDGE_NATIVE_LAB") or "1")
    require(lab in {"0", "1"}, "APPLE_BRIDGE_NATIVE_LAB must be 0 or 1")
    compiler = os.environ.get("APPLE_BRIDGE_SWIFTC", "swiftc")
    sdk = Path(os.environ.get("APPLE_BRIDGE_SDK") or run(str(root / "packaging/dev/sdkroot.sh")) or run("xcrun", "--show-sdk-path"))
    toolchain = {"compiler": shutil.which(compiler), "version": run(compiler, "--version"), "clang_version": run("xcrun", "clang", "--version"), "manifest_compiler": run("xcrun", "--find", "swiftc"), "developer": run("xcode-select", "-p"), "sdk_path": run("xcrun", "--show-sdk-path"), "sdk_version": run("xcrun", "--show-sdk-version")}
    swiftly = Path.home() / ".swiftly/bin/swift"
    if swiftly.exists():
        toolchain["swiftly"] = run(str(swiftly), "--version")
    sdks = {sdk, Path(toolchain["sdk_path"])}
    if os.environ.get("SDKROOT"):
        sdks.add(Path(os.environ["SDKROOT"]))
    for selected in sorted(sdks):
        for name in ["SDKSettings.json", "SDKSettings.plist"]:
            path = selected / name
            if path.exists():
                digest.update(str(path).encode() + b"\0" + path.read_bytes() + b"\0")
    for name in ["APPLE_BRIDGE_SWIFTC", "APPLE_BRIDGE_SDK", "SDKROOT", "DEVELOPER_DIR", "TOOLCHAINS", "APPLE_BRIDGE_SIGNING_GUI", "APPLE_BRIDGE_KEEP_SDKROOT", "APPLE_BRIDGE_NATIVE_TARGET", "SWIFT_EXEC", "CC", "CXX", "CFLAGS", "CXXFLAGS", "SWIFTFLAGS"]:
        toolchain[name] = os.environ.get(name, "")
    toolchain["native_lab_condition"] = lab
    certificate = run("security", "find-certificate", "-c", "Apple Bridge Dev Signing", "-Z", str(Path.home() / "Library/Keychains/apple-bridge-dev.keychain-db"))
    toolchain["signing_certificate"] = hashlib.sha256(certificate.encode()).hexdigest()
    digest.update(json.dumps(toolchain, sort_keys=True).encode())
    return digest.hexdigest()

