import ctypes as c
import hashlib
import json
import os
from pathlib import Path
import plistlib
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]


def run(*args, **kwargs):
    print("running " + str(args[0]), flush=True)
    return subprocess.run([str(a) for a in args], check=True, timeout=600, **kwargs)


def grants(binary):
    result = []
    for path in [Path.home() / "Library/Application Support/com.apple.TCC/TCC.db", Path("/Library/Application Support/com.apple.TCC/TCC.db")]:
        try:
            with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as db:
                rows = db.execute("SELECT service,client,client_type,auth_value FROM access WHERE service='kTCCServiceSystemPolicyAllFiles' AND client IN (?,?,?,?)", (str(binary), str(binary.resolve()), "identity-spikes", "com.johansardar.applebridge.services")).fetchall()
                result.extend(rows)
        except sqlite3.Error:
            result.append(["unavailable"])
    return result


def main():
    os.umask(0o077)
    stage = Path(tempfile.mkdtemp(prefix="ab-identity-spikes-", dir="/tmp"))
    print("stage " + str(stage), flush=True)
    result = {"os": subprocess.check_output(["sw_vers", "-productVersion"], text=True).strip(), "stage": str(stage)}
    manifest = Path(__file__).with_name("Cargo.toml")
    env = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(Path.home() / "ab-cargo-target"))
    run("cargo", "build", "--manifest-path", manifest, env=env)
    binary = stage / "identity-spikes"
    shutil.copy2(Path(env["CARGO_TARGET_DIR"]) / "debug/identity-spikes", binary)
    result["probe_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    app = stage / "AppleBridgeHelper.app"
    run(ROOT / "packaging/dev/build-helper.sh", app, env=dict(os.environ, APPLE_BRIDGE_SIGNED_CACHE_BUILD="1"))
    shutil.copy2(binary, app / "Contents/MacOS/identity-spikes")
    run("codesign", "-d", "--extract-certificates=" + str(stage / "cert"), app)
    identity = hashlib.sha1((stage / "cert0").read_bytes()).hexdigest().upper()
    result["signing_certificate_sha1"] = identity
    run("codesign", "--force", "--sign", identity, "--timestamp=none", app / "Contents/MacOS/identity-spikes")
    shutil.copy2(app / "Contents/MacOS/identity-spikes", binary)
    result["probe_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    signing = ["codesign", "--force", "--sign", identity, "--options", "runtime", "--timestamp=none", "--preserve-metadata=entitlements,requirements", app]
    run(*signing)
    run("codesign", "--verify", "--strict", app)
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    if info["CFBundleIdentifier"] != "com.johansardar.applebridge.services":
        raise RuntimeError("Helper does not match granted services identity")
    result["bundle_id"] = info["CFBundleIdentifier"]
    signature = subprocess.run(["codesign", "-d", "-r-", str(app)], capture_output=True, text=True, check=True)
    result["designated_requirement"] = (signature.stdout + signature.stderr).strip()
    for mode in ["open", "resigned"]:
        if mode == "resigned":
            run(*signing)
            run("codesign", "--verify", "--strict", app)
        output = stage / (mode + ".json")
        run("open", "-n", "-W", "-g", "-a", app, "--args", "lab-tcc-probe", output)
        result[mode] = json.loads(output.read_text())
    label = "dev.applebridge.lab.tcc." + secrets.token_hex(8)
    domain = "gui/" + str(os.getuid())
    job = stage / "control.plist"
    output = stage / "control.json"
    job.write_bytes(plistlib.dumps({"Label": label, "ProgramArguments": [str(binary)], "EnvironmentVariables": {"HOME": str(Path.home())}, "RunAtLoad": True, "StandardOutPath": str(output), "StandardErrorPath": str(stage / "control.stderr")}))
    try:
        run("launchctl", "bootstrap", domain, job)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if output.exists() and output.stat().st_size:
                break
            time.sleep(0.1)
        result["control"] = json.loads(output.read_text())
        signature = subprocess.run(["codesign", "-d", "-r-", str(binary)], capture_output=True, text=True)
        result["control_identity"] = {"label": label, "program": str(binary), "uid": os.getuid(), "signature": (signature.stdout + signature.stderr).strip(), "grants": grants(binary)}
    finally:
        run("launchctl", "bootout", domain + "/" + label)
        result["control_cleaned"] = True
    result_path = stage / "tcc-results.json"
    result_path.write_text(json.dumps(result, indent=2) + "\n")
    if "--tcc-only" in sys.argv:
        Path(sys.argv[1]).write_text(json.dumps(result, indent=2) + "\n")
        return
    security = c.CDLL("/System/Library/Frameworks/Security.framework/Security")
    security.SecKeychainSetUserInteractionAllowed.argtypes = [c.c_bool]
    security.SecKeychainCreate.argtypes = [c.c_char_p, c.c_uint32, c.c_char_p, c.c_bool, c.c_void_p, c.POINTER(c.c_void_p)]
    security.SecKeychainUnlock.argtypes = [c.c_void_p, c.c_uint32, c.c_char_p, c.c_bool]
    security.SecKeychainDelete.argtypes = [c.c_void_p]
    password = secrets.token_hex(32).encode()
    keychain = stage / "spike.keychain-db"
    (stage / "keychain-password").write_bytes(password)
    reference = c.c_void_p()
    if security.SecKeychainSetUserInteractionAllowed(False) != 0:
        raise RuntimeError("Cannot disable Keychain UI")
    status = security.SecKeychainCreate(os.fsencode(keychain), len(password), password, False, None, c.byref(reference))
    if status != 0:
        raise RuntimeError("Throwaway keychain create status " + str(status))
    try:
        status = security.SecKeychainUnlock(reference, len(password), password, True)
        if status != 0:
            raise RuntimeError("Throwaway keychain unlock status " + str(status))
        run("go", "run", "./rust/lab/identity-spikes/go-writer", keychain, cwd=ROOT)
        result["keychain"] = json.loads(subprocess.check_output([str(binary), "keychain", str(keychain)], text=True))
        run("go", "run", "./rust/lab/identity-spikes/go-writer", keychain, "get", cwd=ROOT)
        result["legacy_rollback_read"] = True
    finally:
        result["keychain_delete_status"] = security.SecKeychainDelete(reference)
        (stage / "keychain-password").unlink()
    destination = Path(sys.argv[1]) if len(sys.argv) > 1 else stage / "results.json"
    destination.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
