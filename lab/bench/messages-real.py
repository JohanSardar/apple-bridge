import datetime
import json
import os
import sqlite3
import sys
import time
import urllib.request

result = {"assertions": [], "samples": {}, "status": ""}


def check(name, passed, detail="", pending=False):
    value = {"name": name, "passed": bool(passed)}
    if detail:
        value["detail"] = detail
    if pending:
        value["status"] = "unverified"
        result["status"] = "unverified"
    result["assertions"].append(value)


def readonly(path):
    return sqlite3.connect("file:" + path + "?mode=ro", uri=True, timeout=5)


def api(endpoint, query):
    request = urllib.request.Request(endpoint["URL"] + "/v1/op/records.query", json.dumps(query).encode(), {"Authorization": "Bearer " + endpoint["Token"], "Content-Type": "application/json", "Host": endpoint["Host"]})
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)["records"]


def main():
    with open(sys.argv[1]) as source:
        config = json.load(source)
    endpoint = config["gateway"]
    native = readonly(os.path.expanduser("~/Library/Messages/chat.db"))
    check("truth_readonly_chat_db", native.execute("PRAGMA query_only=ON") is not None)
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        sources = api(endpoint, {"collection": "system.source", "limit": 100})
        message_sources = [row["data"] for row in sources if row["data"].get("source") == "messages"]
        if message_sources and message_sources[0].get("state") == "ready":
            break
        time.sleep(0.5)
    else:
        check("messages_source_ready", False, "Helper Messages access unavailable; inspect permissions without prompting", True)
        return
    check("messages_source_ready", True)
    state = readonly(os.path.join(endpoint["Home"], "state.db"))
    scenario = config["scenario"]
    if scenario == "MSG-HISTORY-IMPORT":
        started = time.monotonic()
        deadline = started + 1000
        equal = False
        initial_position = native.execute("SELECT MAX(ROWID) FROM message").fetchone()[0]
        while time.monotonic() < deadline:
            expected = dict(native.execute("SELECT COALESCE(c.guid,''),COUNT(DISTINCT m.guid) FROM message m LEFT JOIN chat_message_join j ON j.message_id=m.ROWID LEFT JOIN chat c ON c.ROWID=j.chat_id WHERE COALESCE(m.associated_message_type,0) NOT BETWEEN 2000 AND 2007 AND COALESCE(m.associated_message_type,0) NOT BETWEEN 3000 AND 3007 GROUP BY COALESCE(c.guid,'')"))
            aliases = dict(state.execute("SELECT record_id,native_key FROM aliases WHERE collection='messages.chat'"))
            observed = {}
            for chat_id, count in state.execute("SELECT COALESCE(json_extract(payload,'$.data.chat_id'),''),COUNT(*) FROM records WHERE collection='messages.message' AND deleted=0 GROUP BY COALESCE(json_extract(payload,'$.data.chat_id'),'')"):
                chat = aliases.get(chat_id, "" if chat_id == "" else None)
                observed[chat] = count
            if observed == expected:
                equal = True
                break
            time.sleep(5)
        check("gateway_per_chat_counts_equal_readonly_truth", equal)
        result["samples"]["messages_ready_to_gateway_history_equal"] = [(time.monotonic() - started) * 1000]
        changed = native.execute("SELECT MAX(ROWID) FROM message").fetchone()[0] > initial_position
        check("live_changes_during_import_revalidated", equal and changed, "No natural insert observed during import; simulator covers concurrent mutation" if not changed else "", not changed)
    elif scenario == "MSG-RECEIPTS":
        row = native.execute("SELECT guid,date_delivered,date_read FROM message WHERE is_from_me=1 AND (date_delivered>0 OR date_read>0) ORDER BY ROWID DESC LIMIT 1").fetchone()
        if row is None:
            check("receipt_truth_available", False, "No existing outgoing receipt available", True)
            return
        guid, delivered, read = row
        deadline = time.monotonic() + 120
        matched = False
        while time.monotonic() < deadline:
            alias = state.execute("SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", (guid,)).fetchone()
            if alias:
                records = api(endpoint, {"collection": "messages.message", "id": alias[0], "limit": 1})
                if records:
                    def stamp(value):
                        if not value:
                            return None
                        seconds = value / 1e9 if value > 1e12 else value
                        return datetime.datetime(2001, 1, 1, tzinfo=datetime.timezone.utc) + datetime.timedelta(seconds=seconds)
                    data = records[0]["data"]
                    matched = all((data.get(field) is None if not raw else data.get(field) is not None and abs((datetime.datetime.fromisoformat(data[field].replace("Z", "+00:00")) - stamp(raw)).total_seconds()) < 0.002) for field, raw in [("delivered_at", delivered), ("read_at", read)])
                    if matched:
                        break
            time.sleep(0.25)
        check("existing_outgoing_receipts_equal_readonly_truth", matched)
        check("fresh_self_receipt_transition", False, "Existing receipt mapping checked; fresh self-send pending unlocked screen", True)
    elif scenario == "MSG-WATCHER-REARM":
        diagnostic = os.path.join(endpoint["Home"], "helper", "messages-watch.json")
        deadline = time.monotonic() + 40
        stats = {}
        while time.monotonic() < deadline:
            try:
                with open(diagnostic) as source:
                    stats = json.load(source)
            except (OSError, ValueError):
                pass
            if stats.get("rearms", 0) > 0:
                break
            time.sleep(0.25)
        check("native_watchers_armed", stats.get("watched_paths", 0) >= 3)
        check("poll_fallback_active", stats.get("polls", 0) > 0)
        check("natural_inode_rearm_observed", stats.get("rearms", 0) > 0, "No natural WAL inode replacement observed; chat.db was never modified" if not stats.get("rearms", 0) else "", not stats.get("rearms", 0))
    elif scenario == "MSG-INCOMING":
        position = native.execute("SELECT MAX(ROWID) FROM message").fetchone()[0]
        deadline = time.monotonic() + 60
        samples = 0
        while time.monotonic() < deadline:
            rows = native.execute("SELECT ROWID,guid FROM message WHERE ROWID>? AND COALESCE(associated_message_type,0) NOT BETWEEN 2000 AND 2007 AND COALESCE(associated_message_type,0) NOT BETWEEN 3000 AND 3007 ORDER BY ROWID", (position,)).fetchall()
            at = time.monotonic()
            for position, guid in rows:
                end = at + 5
                gateway_at = None
                replica_at = None
                while time.monotonic() < end:
                    alias = state.execute("SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", (guid,)).fetchone()
                    if alias:
                        query = {"collection": "messages.message", "id": alias[0], "limit": 1}
                        if gateway_at is None and api(endpoint, query):
                            gateway_at = time.monotonic()
                        if config["replicas"] and replica_at is None and api(config["replicas"][0], query):
                            replica_at = time.monotonic()
                        if gateway_at and (not config["replicas"] or replica_at):
                            break
                    time.sleep(0.01)
                check("natural_insert_reaches_gateway", gateway_at is not None)
                if gateway_at:
                    result["samples"].setdefault("chat_db_row_first_seen_to_gateway", []).append(max(0, (gateway_at - at) * 1000))
                if config["replicas"]:
                    check("natural_insert_reaches_replica_under_one_second", replica_at is not None and replica_at - at < 1)
                    if replica_at:
                        result["samples"].setdefault("chat_db_row_first_seen_to_replica", []).append(max(0, (replica_at - at) * 1000))
                        result["samples"].setdefault("gateway_to_replica", []).append(max(0, (replica_at - gateway_at) * 1000))
                samples += 1
            if samples:
                break
            time.sleep(0.01)
        if not samples:
            check("fresh_row_latency", False, "No natural message row observed; self-send pending unlocked screen", True)


try:
    main()
except Exception as error:
    check("readonly_truth_execution", False, "Private diagnostic suppressed; error type " + type(error).__name__)
print(json.dumps(result))
