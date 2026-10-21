#!/usr/bin/env python3
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from signed_app_inputs import digest_bundle, require, source_key

root = Path(__file__).resolve().parents[2]
cache = Path(os.environ.get("APPLE_BRIDGE_SIGNED_APP_CACHE", str(Path.home() / "Library/Caches/AppleBridgeDev/signed-apps")))
cache.mkdir(parents=True, exist_ok=True, mode=0o700)

@contextmanager
def locked(key=None, blocking=True):
    name = ".lock" if key is None else ".key-" + key + ".lock"
    with (cache / name).open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
        except BlockingIOError:
            yield None
            return
        yield lock

def trim():
    for stage in cache.glob(".stage-*"):
        key = stage.name[7:71]
        if len(key) == 64 and all(c in "0123456789abcdef" for c in key):
            with locked(key, blocking=False) as owner:
                if owner is not None:
                    shutil.rmtree(stage)
        else:
            shutil.rmtree(stage)
    entries = sorted((p for p in cache.iterdir() if p.is_dir() and len(p.name) == 64), key=lambda p: p.stat().st_mtime, reverse=True)
    for entry in entries[3:]:
        with locked(entry.name, blocking=False) as owner:
            if owner is not None:
                shutil.rmtree(entry)

if sys.argv[1:] == ["--trim"]:
    with locked(blocking=False) as index:
        if index is not None:
            trim()
    raise SystemExit(0)

role, destination = sys.argv[1], Path(sys.argv[2]).resolve()
names = {"helper": "AppleBridgeHelper.app", "truth": "AppleTruth.app"}
require(role in names and not destination.exists(), "new immutable destination required")
started = time.monotonic()
key = source_key(root)
entry = cache / key
app, proof = entry / names[role], entry / (role + ".json")
with locked(key) as owner:
    hit = app.exists() and proof.exists()
    recovered = False
    try:
        require(hit or not (app.exists() or proof.exists()), "incomplete cached bundle")
        if hit:
            subprocess.run(["codesign", "--verify", "--strict", str(app)], check=True)
            record = json.loads(proof.read_text())
            require(record["key"] == key and digest_bundle(app) == record["bundle_sha256"], "cached bundle changed")
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError):
        with locked():
            shutil.rmtree(app, ignore_errors=True)
            proof.unlink(missing_ok=True)
        hit = False
        recovered = True
    if not hit:
        with locked():
            stage = Path(tempfile.mkdtemp(prefix=".stage-" + key + "-", dir=cache))
        try:
            environment = dict(os.environ, APPLE_BRIDGE_SIGNED_CACHE_BUILD="1")
            subprocess.run([str(root / "packaging/dev" / ("build-" + role + ".sh")), str(stage / names[role])], env=environment, check=True, pass_fds=(owner.fileno(),))
            require(source_key(root) == key, "native build inputs changed during build")
            subprocess.run(["codesign", "--verify", "--strict", str(stage / names[role])], check=True)
            bundle_hash = digest_bundle(stage / names[role])
            with locked():
                entry.mkdir(mode=0o700, exist_ok=True)
                os.rename(stage / names[role], app)
                proof.write_text(json.dumps({"key": key, "bundle_sha256": bundle_hash}))
                proof.chmod(0o600)
        finally:
            with locked():
                shutil.rmtree(stage)
    subprocess.run(["codesign", "--verify", "--strict", str(app)], check=True)
    expected = json.loads(proof.read_text())["bundle_sha256"]
    require(digest_bundle(app) == expected, "cached bundle changed")
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["ditto", str(app), str(destination)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(destination)], check=True)
    require(digest_bundle(destination) == expected, "staged bundle differs")
    with locked():
        os.utime(entry, None)
with locked(blocking=False) as index:
    if index is not None:
        trim()
result = {"signed_app_cache": role, "hit": hit, "recovered_corrupt_entry": recovered, "key": key, "bundle_sha256": expected, "setup_ms": round((time.monotonic() - started) * 1000, 3)}
report = destination.parent / (destination.stem + ".cache.json")
report.write_text(json.dumps(result))
report.chmod(0o600)
print(json.dumps(result))
