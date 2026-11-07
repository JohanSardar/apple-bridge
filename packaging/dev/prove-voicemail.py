import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import uuid


def signature(path):
    try:
        info = path.stat()
        return info.st_ino, info.st_size, info.st_mtime_ns
    except FileNotFoundError:
        return None


def prove(binary):
    root = Path.home() / "Library/Group Containers/group.com.apple.FaceTime/com.apple.facetimemessagestored/Data Store"
    source = root / "FaceTimeMessageStore-local.sqlitedb"
    if not source.exists():
        return {"status": "unverified", "reason": "store_absent"}
    with tempfile.TemporaryDirectory(prefix="ab-voicemail-proof-") as temporary:
        private = Path(temporary)
        snapshot = private / "store"
        snapshot.mkdir()
        files = [Path(str(source) + suffix) for suffix in ("", "-wal")]
        before = [signature(path) for path in files]
        for original in files:
            if original.exists():
                shutil.copyfile(original, snapshot / original.name)
        if before != [signature(path) for path in files]:
            return {"status": "unverified", "reason": "store_changed_during_snapshot"}
        database = snapshot / source.name
        connection = sqlite3.connect(database)
        columns = {row[1] for row in connection.execute("PRAGMA table_info(ZSTOREDMESSAGE)")}
        required = {"ZTRANSCRIPT", "ZFILETYPE", "ZRECORDUUID", "ZDATEDELETED"}
        if not required.issubset(columns):
            connection.close()
            return {"status": "unverified", "reason": "proof_schema_unavailable"}
        rows = connection.execute("SELECT ZRECORDUUID,ZFILETYPE,ZTRANSCRIPT,ZPROVIDER,ZMESSAGETYPE FROM ZSTOREDMESSAGE WHERE ZDATEDELETED IS NULL").fetchall()
        connection.close()
        expected = {"messages": len(rows), "transcripts": sum(bool(row[2]) for row in rows), "audio": 0}
        hashes = set()
        for identity, extension, _, provider, kind in rows:
            if not extension or extension.lower() not in ("amr", "m4a", "mov"):
                continue
            if (provider or "").lower().endswith(".facetimeprovider") and kind == 2:
                continue
            try:
                identity = str(uuid.UUID(bytes=identity) if isinstance(identity, bytes) else uuid.UUID(identity)).upper()
            except (ValueError, AttributeError, TypeError):
                continue
            relative = Path("Assets") / identity[:2] / (identity + "." + extension)
            original = root / relative
            if not original.is_file() or not original.resolve().is_relative_to(root.resolve()):
                continue
            if getattr(original.stat(), "st_flags", 0) & 0x40000000:
                continue
            prior = signature(original)
            destination = snapshot / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            with original.open("rb") as audio:
                hash_value = hashlib.sha256()
                for block in iter(lambda: audio.read(131_072), b""):
                    hash_value.update(block)
                digest = hash_value.hexdigest()
            shutil.copyfile(original, destination)
            if prior != signature(original):
                return {"status": "unverified", "reason": "audio_changed_during_snapshot"}
            hashes.add(digest)
            expected["audio"] += 1
        if not expected["messages"]:
            return {"status": "unverified", "reason": "no_real_voicemails", "store": expected}
        result = subprocess.run([str(binary), str(database), str(private / "staging")], capture_output=True, check=False)
        if result.returncode:
            return {"status": "unverified", "reason": "reader_failed", "exit_code": result.returncode}
        actual = json.loads(result.stdout)
        count_match = all(actual[key] == expected[key] for key in expected)
        hash_match = bool(actual["fetched_sha256"]) and actual["fetched_sha256"] in hashes
        return {"status": "passed" if count_match and hash_match else "failed", "store": expected,
                "reader": {key: actual[key] for key in expected}, "counts_match": count_match,
                "fetched_sha256_matches_original": hash_match}


def main():
    parser = argparse.ArgumentParser(description="Read-only voicemail count/hash proof; emits no personal content.")
    parser.add_argument("reader", type=Path)
    arguments = parser.parse_args()
    os.umask(0o077)
    try:
        result = prove(arguments.reader.resolve())
    except (OSError, sqlite3.Error, ValueError, KeyError) as error:
        result = {"status": "unverified", "reason": type(error).__name__}
    print(json.dumps(result, sort_keys=True))
    raise SystemExit(0 if result["status"] == "passed" else 1)


if __name__ == "__main__":
    main()
