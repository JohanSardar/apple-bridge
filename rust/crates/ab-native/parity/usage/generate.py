import json
import pathlib
import sqlite3
import struct
import sys

root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
(root / "sync").mkdir()
with sqlite3.connect(root / "sync/sync.db") as db:
    db.execute("CREATE TABLE DevicePeer(device_identifier TEXT,name TEXT,platform INTEGER,me INTEGER,model TEXT)")
    db.executemany("INSERT INTO DevicePeer VALUES(?,?,?,?,?)", [
        ("synthetic-local", "Synthetic Mac", 3, 1, "Mac"),
        ("synthetic-remote", "", 2, 0, "iPad"),
        ("missing-remote", "Missing", 2, 0, "iPhone"),
    ])
base = root / "streams/restricted/App.InFocus"
start = 1800100800.0

def segment(events):
    data = b"SEGB-synthetic"
    for offset, starting, bundle in events:
        bundle = bundle.encode()
        data += bytes([0x10, 1, 0x18, starting, 0x21]) + struct.pack("<d", start + offset - 978307200) + bytes([0x32, len(bundle)]) + bundle
    return data

for directory, events in [
    ("local", [(0, 1, "test.synthetic.alpha"), (60, 0, "test.synthetic.alpha"), (120, 1, "com.apple.universalcontrol"), (180, 1, "test.synthetic.beta"), (300, 1, "loginwindow"), (600, 0, "test.synthetic.beta")]),
    ("remote/synthetic-remote", [(0, 1, "test.synthetic.ipad"), (20000, 0, "test.synthetic.ipad")]),
]:
    path = base / directory
    path.mkdir(parents=True)
    (path / "1").write_bytes(segment(events))
    (path / "ignored.segb").write_bytes(segment([(0, 1, "ignored"), (60, 0, "ignored")]))
    (path / ".hidden").mkdir()
    (path / ".hidden/2").write_bytes(segment(events))
    (path / "tombstone").mkdir()
    (path / "tombstone/3").write_bytes(segment([(0, 1, "deleted"), (60, 0, "deleted")]))
with sqlite3.connect(root / "knowledgeC.db") as db:
    db.execute("CREATE TABLE ZSOURCE(Z_PK INTEGER,ZDEVICEID TEXT)")
    db.execute("INSERT INTO ZSOURCE VALUES(1,'synthetic-hardware'),(2,'other-hardware')")
    db.execute("CREATE TABLE ZOBJECT(ZSTARTDATE REAL,ZENDDATE REAL,ZSTREAMNAME TEXT,ZVALUEINTEGER INTEGER,ZSOURCE INTEGER)")
    db.executemany("INSERT INTO ZOBJECT VALUES(?,?,?,?,?)", [
        (start + 10 - 978307200, start + 30 - 978307200, '/device/isLocked', 1, 1),
        (start + 20 - 978307200, start + 40 - 978307200, '/display/isBacklit', 0, None),
        (start - 978307200, start + 100 - 978307200, '/device/isLocked', 1, 2),
        (start + 80 - 978307200, start + 70 - 978307200, '/device/isLocked', 1, 1),
    ])
(root / "context.json").write_text(json.dumps({
    "generation": "synthetic-generation", "checkpoint": "synthetic-checkpoint", "scan_id": "synthetic-scan", "observed_at": "2027-01-16T12:00:00.000Z", "zone": "America/New_York", "local_name": "Synthetic Mac", "blocked": [], "power": [],
    "history": {"database": "knowledgeC.db", "hardware_id": "synthetic-hardware", "now": start + 600},
}))
