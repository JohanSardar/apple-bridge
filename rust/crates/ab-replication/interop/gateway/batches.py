import json
import pathlib
import sqlite3
import sys

path = pathlib.Path(sys.argv[1]).resolve().as_uri() + "?mode=ro"
with sqlite3.connect(path, uri=True) as connection:
    batches = connection.execute(
        "SELECT after_seq,through_seq FROM applied_batches ORDER BY after_seq,through_seq"
    ).fetchall()
if batches != [(1, 2)]:
    raise SystemExit(f"batch sequence mismatch: {batches}")
print(json.dumps(batches))
