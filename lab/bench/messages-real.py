import datetime
import json
import os
import sqlite3
import sys
import time
import urllib.request
import urllib.error
import traceback

result = {"assertions": [], "samples": {}, "status": "", "metrics": {"read_only": True, "natural_events_only": True, "truth_poll_interval_ms": 25}}


def check(name, passed, detail="", pending=False):
    value = {"name": name, "passed": bool(passed)}
    if detail:
        value["detail"] = detail
    if pending:
        value["status"] = "unverified"
        result["status"] = "unverified"
    result["assertions"].append(value)


class ReadOnlyDatabase:
    def __init__(self, path):
        self.path = path
        self.connection = None
        self.identity = None

    @staticmethod
    def file_identity(path):
        try:
            value = os.stat(path)
            return value.st_dev, value.st_ino
        except FileNotFoundError:
            return None

    def execute(self, sql, bindings=()):
        current = self.file_identity(self.path), self.file_identity(self.path + "-wal")
        if self.connection is None or current[0] != self.identity[0] or current[1] is not None and current[1] != self.identity[1]:
            if self.connection is not None:
                self.connection.close()
            self.connection = sqlite3.connect("file:" + self.path + "?mode=ro", uri=True, timeout=5)
            self.connection.execute("PRAGMA query_only=ON")
            self.identity = self.file_identity(self.path), self.file_identity(self.path + "-wal")
        return self.connection.execute(sql, bindings)


def readonly(path):
    return ReadOnlyDatabase(path)


def api(endpoint, query):
    request = urllib.request.Request(endpoint["URL"] + "/v1/op/records.query", json.dumps(query).encode(), {"Authorization": "Bearer " + endpoint["Token"], "Content-Type": "application/json", "Host": endpoint["Host"]})
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            return json.load(response)["records"]
    except urllib.error.HTTPError as error:
        if error.code == 503 or error.code == 404 and query.get("id"):
            counts = result["metrics"].setdefault("api_visibility_retries", {})
            key = str(error.code)
            counts[key] = counts.get(key, 0) + 1
            return []
        raise


def receipt_values(native, guid):
    row = native.execute("SELECT date_delivered,date_read FROM message WHERE guid=?", (guid,)).fetchone()
    return tuple(value if value and value > 0 else None for value in row) if row else (None, None)


def receipt_snapshot(native):
    return {guid: tuple(value if value and value > 0 else None for value in (delivered, read)) for guid, delivered, read in native.execute("SELECT guid,date_delivered,date_read FROM message WHERE is_from_me=1 ORDER BY ROWID DESC LIMIT 2000")}


def receipt_matches(data, values):
    for field, raw in zip(["delivered_at", "read_at"], values):
        if not raw:
            if data.get(field) is not None:
                return False
            continue
        if data.get(field) is None:
            return False
        seconds = raw / 1e9 if raw > 1e12 else raw
        expected = datetime.datetime(2001, 1, 1, tzinfo=datetime.timezone.utc) + datetime.timedelta(seconds=seconds)
        actual = datetime.datetime.fromisoformat(data[field].replace("Z", "+00:00"))
        if abs((actual - expected).total_seconds()) >= 0.002:
            return False
    return True


def wait_visible(state, endpoints, guid, predicate=lambda data: True, timeout=5):
    deadline = time.monotonic() + timeout
    visible = [None] * len(endpoints)
    while time.monotonic() < deadline:
        alias = state.execute("SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", (guid,)).fetchone()
        if alias:
            for index, endpoint in enumerate(endpoints):
                if visible[index] is not None:
                    continue
                rows = api(endpoint, {"collection": "messages.message", "id": alias[0], "limit": 1})
                if rows and predicate(rows[0]["data"]):
                    visible[index] = time.monotonic()
            if all(at is not None for at in visible):
                break
        time.sleep(0.01)
    return visible


def wait_receipts_visible(native, state, endpoints, guid, timeout=5):
    deadline = time.monotonic() + timeout
    last = [None] * len(endpoints)
    while time.monotonic() < deadline:
        expected = receipt_values(native, guid)
        last = wait_visible(state, endpoints, guid, lambda data: receipt_matches(data, expected), min(1, deadline - time.monotonic()))
        if receipt_values(native, guid) != expected:
            last = [None] * len(endpoints)
        elif all(at is not None for at in last):
            return last
    return last


def wal_state(path):
    try:
        value = os.stat(path)
        with open(path, "rb") as source:
            header = source.read(32)
        return (value.st_dev, value.st_ino), value.st_size, header[16:24] if len(header) == 32 else None
    except FileNotFoundError:
        return None, 0, None


def watcher_stats(path):
    try:
        with open(path) as source:
            return json.load(source)
    except (OSError, ValueError):
        return {}


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
        check("messages_source_ready", False, "Helper did not publish Messages ready; startup and permission status unverified", True)
        return
    check("messages_source_ready", True)
    state = readonly(os.path.join(endpoint["Home"], "state.db"))
    scenario = config["scenario"]
    endpoints = [endpoint] + (config["replicas"] or [])
    result["metrics"]["replica_count"] = len(config["replicas"] or [])
    if scenario == "MSG-HISTORY-IMPORT":
        started = time.monotonic()
        deadline = started + 1000
        equal = False
        initial_position = native.execute("SELECT COALESCE(MAX(ROWID),0) FROM message").fetchone()[0]
        initial_observed = None
        if config.get("history_started"):
            result["metrics"].update(natural_events_only=False, driven_by="guarded_self_send_during_history_import")
            with open(config["history_started"], "w") as marker:
                marker.write("ready")
        while time.monotonic() < deadline:
            expected = dict(native.execute("SELECT COALESCE(c.guid,''),COUNT(DISTINCT m.guid) FROM message m LEFT JOIN chat_message_join j ON j.message_id=m.ROWID LEFT JOIN chat c ON c.ROWID=j.chat_id WHERE COALESCE(m.associated_message_type,0) NOT BETWEEN 2000 AND 2007 AND COALESCE(m.associated_message_type,0) NOT BETWEEN 3000 AND 3007 GROUP BY COALESCE(c.guid,'')"))
            aliases = dict(state.execute("SELECT record_id,native_key FROM aliases WHERE collection='messages.chat'"))
            observed = {}
            for chat_id, count in state.execute("SELECT COALESCE(json_extract(payload,'$.data.chat_id'),''),COUNT(*) FROM records WHERE collection='messages.message' AND deleted=0 GROUP BY COALESCE(json_extract(payload,'$.data.chat_id'),'')"):
                chat = aliases.get(chat_id, "" if chat_id == "" else None)
                observed[chat] = count
            if initial_observed is None:
                initial_observed = sum(observed.values())
            if observed == expected:
                equal = True
                break
            time.sleep(5)
        check("gateway_per_chat_counts_equal_readonly_truth", equal)
        elapsed = time.monotonic() - started
        result["metrics"].update(history_complete=equal, history_observation_ms=elapsed * 1000, history_expected_message_records=sum(expected.values()), history_observed_message_records=sum(observed.values()), history_initial_observed_message_records=initial_observed, history_observed_rows_per_second=(sum(observed.values()) - initial_observed) / elapsed, history_mismatched_chat_count=sum(expected.get(key, 0) != observed.get(key, 0) for key in expected.keys() | observed.keys()))
        if equal:
            result["samples"]["messages_ready_to_gateway_history_equal"] = [elapsed * 1000]
        changed = native.execute("SELECT COALESCE(MAX(ROWID),0) FROM message").fetchone()[0] > initial_position
        check("live_changes_during_import_revalidated", equal and changed, "No natural insert observed during import; simulator covers concurrent mutation" if not changed else "", not changed)
    elif scenario == "MSG-RECEIPTS":
        baseline = receipt_snapshot(native)
        row = native.execute("SELECT guid FROM message WHERE is_from_me=1 AND (date_delivered>0 OR date_read>0) ORDER BY ROWID DESC LIMIT 1").fetchone()
        if row is None:
            check("receipt_truth_available", False, "No existing outgoing receipt available", True)
            return
        guid = row[0]
        visible = wait_receipts_visible(native, state, endpoints, guid, 120)
        check("existing_outgoing_receipts_equal_readonly_truth", visible[0] is not None)
        check("existing_receipts_equal_on_replica", len(visible) > 1 and all(at is not None for at in visible[1:]))
        baseline = receipt_snapshot(native)
        started = time.monotonic()
        deadline = started + 60
        changes = 0
        while time.monotonic() < deadline:
            current = receipt_snapshot(native)
            changed = [key for key in current if key in baseline and current[key] != baseline[key]]
            detected = time.monotonic()
            for key in changed:
                seen = wait_receipts_visible(native, state, endpoints, key)
                check("natural_receipt_transition_matches_gateway", seen[0] is not None)
                check("natural_receipt_transition_matches_replica", len(seen) > 1 and all(at is not None for at in seen[1:]))
                for index, at in enumerate(seen):
                    if at is not None:
                        name = "receipt_row_first_seen_to_gateway" if index == 0 else "receipt_row_first_seen_to_replica"
                        result["samples"].setdefault(name, []).append(max(0, (at - detected) * 1000))
                changes += 1
            if changes:
                break
            time.sleep(0.025)
        result["metrics"].update(receipt_transition_samples=changes, natural_receipt_observation_ms=(time.monotonic() - started) * 1000)
        check("natural_receipt_transition_observed", changes > 0, "No natural receipt transition observed during the bounded read-only window" if not changes else "", not changes)
    elif scenario == "MSG-WATCHER-REARM":
        diagnostic = os.path.join(endpoint["Home"], "helper", "messages-watch.json")
        wal = os.path.expanduser("~/Library/Messages/chat.db-wal")
        before = watcher_stats(diagnostic)
        previous = wal_state(wal)
        started = time.monotonic()
        deadline = started + 90
        inode_changes, truncations, header_resets = 0, 0, 0
        after = before
        position_after_checkpoint = None
        captures_after_checkpoint = 0
        while time.monotonic() < deadline:
            current = wal_state(wal)
            inode_changes += int(current[0] != previous[0])
            truncations += int(previous[1] > 0 and current[1] < previous[1])
            header_resets += int(current[0] == previous[0] and current[2] is not None and previous[2] is not None and current[2] != previous[2])
            previous = current
            after = watcher_stats(diagnostic)
            if inode_changes or truncations or header_resets:
                if position_after_checkpoint is None:
                    position_after_checkpoint = native.execute("SELECT COALESCE(MAX(ROWID),0) FROM message").fetchone()[0]
                else:
                    rows = native.execute("SELECT ROWID,guid FROM message WHERE ROWID>? AND COALESCE(associated_message_type,0) NOT BETWEEN 2000 AND 2007 AND COALESCE(associated_message_type,0) NOT BETWEEN 3000 AND 3007 ORDER BY ROWID", (position_after_checkpoint,)).fetchall()
                    detected = time.monotonic()
                    for position_after_checkpoint, key in rows:
                        visible = wait_visible(state, endpoints, key)
                        check("natural_change_after_checkpoint_captured", all(at is not None for at in visible))
                        if all(at is not None for at in visible):
                            result["samples"].setdefault("post_checkpoint_row_first_seen_to_gateway", []).append(max(0, (visible[0] - detected) * 1000))
                        captures_after_checkpoint += 1
            time.sleep(0.025)
        polls = after.get("polls", 0) - before.get("polls", 0)
        events = after.get("events", 0) - before.get("events", 0)
        rearms = after.get("rearms", 0) - before.get("rearms", 0)
        checkpoint = inode_changes > 0 or truncations > 0 or header_resets > 0
        result["metrics"].update(natural_wal_observation_ms=(time.monotonic() - started) * 1000, wal_inode_changes=inode_changes, wal_truncations=truncations, wal_header_resets=header_resets, watcher_poll_delta=polls, watcher_event_delta=events, watcher_rearm_delta=rearms, natural_post_checkpoint_samples=captures_after_checkpoint)
        check("natural_watchers_remain_armed", before.get("watched_paths", 0) >= 3 and after.get("watched_paths", 0) >= 3)
        check("poll_fallback_active_during_observation", polls > 0)
        check("natural_wal_checkpoint_observed", checkpoint, "No natural WAL checkpoint observed; chat.db and WAL were never modified by this run" if not checkpoint else "", not checkpoint)
        check("natural_post_checkpoint_change_observed", captures_after_checkpoint > 0, "No natural new row after a checkpoint observed during this run" if not captures_after_checkpoint else "", not captures_after_checkpoint)
        if inode_changes:
            check("watcher_rearmed_after_observed_inode_change", rearms > 0 and events > 0)
        else:
            check("natural_inode_rearm_observed", False, "No natural WAL inode replacement observed during the bounded read-only window", True)
    elif scenario == "MSG-INCOMING":
        position = native.execute("SELECT COALESCE(MAX(ROWID),0) FROM message").fetchone()[0]
        deadline = time.monotonic() + 60
        samples = 0
        started = time.monotonic()
        while time.monotonic() < deadline:
            rows = native.execute("SELECT ROWID,guid FROM message WHERE ROWID>? AND COALESCE(is_from_me,0)=0 AND COALESCE(associated_message_type,0) NOT BETWEEN 2000 AND 2007 AND COALESCE(associated_message_type,0) NOT BETWEEN 3000 AND 3007 ORDER BY ROWID", (position,)).fetchall()
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
                check("natural_insert_reaches_gateway_under_one_second", gateway_at is not None and gateway_at - at < 1)
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
            time.sleep(0.025)
        result["metrics"].update(natural_incoming_samples=samples, natural_incoming_observation_ms=(time.monotonic() - started) * 1000)
        if not samples:
            check("fresh_row_latency", False, "No natural message row observed during this read-only run", True)


try:
    main()
except Exception as error:
    result["metrics"]["error_type"] = type(error).__name__
    frames = traceback.extract_tb(error.__traceback__)
    result["metrics"]["error_frames"] = [{"file": os.path.basename(frame.filename), "line": frame.lineno, "function": frame.name} for frame in frames]
    if isinstance(error, urllib.error.HTTPError):
        result["metrics"]["http_status"] = error.code
    check("readonly_truth_execution", False, "Private diagnostic suppressed; error type " + type(error).__name__)
if any(not item["passed"] and item.get("status") != "unverified" for item in result["assertions"]):
    result["status"] = "failed"
print(json.dumps(result))
