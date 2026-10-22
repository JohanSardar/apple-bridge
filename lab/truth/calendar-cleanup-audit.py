import json
import pathlib
import sqlite3
import sys
import time

path = pathlib.Path.home() / "Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb"
connection = sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)
connection.execute("PRAGMA query_only = ON")
connection.execute("BEGIN")
if "--tag" in sys.argv:
    tag = sys.argv[sys.argv.index("--tag") + 1]
    if not tag.startswith("[ab-test ") or not tag.endswith("]") or "\n" in tag:
        raise SystemExit("invalid fixture tag")
    remaining = connection.execute("SELECT count(*) FROM CalendarItem WHERE substr(summary,1,length(?))=?", (tag, tag)).fetchone()[0]
    counts = connection.execute("SELECT c.UUID,count(i.ROWID),sum(CASE WHEN substr(i.summary,1,length(?))=? THEN 1 ELSE 0 END) FROM Calendar c LEFT JOIN CalendarItem i ON c.ROWID=i.calendar_id WHERE c.title=? GROUP BY c.ROWID", (tag, tag, "Bridge Test " + tag)).fetchall()
    connection.rollback()
    connection.close()
    print(json.dumps({"tag": tag, "all_dates": True, "read_only": True, "generated_at": time.time(), "remaining": remaining, "counts": {row[0]: row[1] for row in counts}, "tagged_counts": {row[0]: row[2] for row in counts}}))
    raise SystemExit(0)
tagged = connection.execute("SELECT count(*) FROM CalendarItem WHERE summary LIKE '[ab-test%'").fetchone()[0]
calendars = connection.execute("SELECT count(*) FROM Calendar WHERE title = 'Bridge Test'").fetchone()[0]
items = connection.execute("SELECT count(*) FROM CalendarItem JOIN Calendar ON Calendar.ROWID = CalendarItem.calendar_id WHERE Calendar.title = 'Bridge Test'").fetchone()[0]
bridge_tagged = connection.execute("SELECT count(*) FROM CalendarItem JOIN Calendar ON Calendar.ROWID = CalendarItem.calendar_id WHERE Calendar.title = 'Bridge Test' AND CalendarItem.summary LIKE '[ab-test%'").fetchone()[0]
empty_proof = None
if "--empty-proof" in sys.argv:
    rows = connection.execute("SELECT c.UUID,count(i.ROWID) FROM Calendar c LEFT JOIN CalendarItem i ON c.ROWID=i.calendar_id WHERE c.title='Bridge Test' GROUP BY c.ROWID").fetchall()
    empty_proof = {"all_dates": True, "read_only": True, "generated_at": time.time(), "counts": dict(rows)}
connection.rollback()
connection.close()
if empty_proof is not None:
    print(json.dumps(empty_proof))
    raise SystemExit(0)
clean = tagged == 0 and items == 0 and calendars <= 1
print(json.dumps({"oracle": "independent_read_only_calendar_database", "all_dates": True, "tagged_items": tagged, "bridge_test_calendars": calendars, "bridge_test_items": items, "bridge_test_tagged_items": bridge_tagged, "passed": clean}, indent=2))
if "--assert-clean" in sys.argv and not clean:
    raise SystemExit(1)
