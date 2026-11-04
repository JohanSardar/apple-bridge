import glob, json, os, random, secrets, signal, sqlite3, subprocess, sys, time, urllib.error, urllib.request

app, bridge, source = sys.argv[1], sys.argv[2], sys.argv[3]
label = sys.argv[4] if len(sys.argv) > 4 else "run"
base_dir = os.environ.get("REAL_WORLD_ROOT", "/tmp/ab-real-native")
world = "%s/w-%s-%s-%s" % (base_dir, source, label, secrets.token_hex(4))
os.makedirs(world, mode=0o700)
gateway_home = world + "/gateway"
os.makedirs(gateway_home + "/helper", mode=0o700)
with open(gateway_home + "/helper/lab-capture-source", "w") as handle:
    handle.write(source)
report_dir = os.path.expanduser("~/Library/Application Support/AppleBridgeDev")
env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE="file")
HOME = os.path.expanduser("~")
RUST_ROOTS = {
    "safari": HOME + "/Library/Safari",
    "phone": HOME + "/Library/Application Support/CallHistoryDB",
    "mail": HOME + "/Library/Mail/V10/MailData",
    "usage": HOME + "/Library/Biome",
    "notes": HOME + "/Library/Group Containers/group.com.apple.notes",
}
FACTS_SOURCES = ("calendar", "contacts", "reminders")
# Live capture owners: REAL_RUST_SOURCES lists what ab-session captures, the way the live switch file does. Empty keeps the Swift path.
rust_sources = [name for name in os.environ.get("REAL_RUST_SOURCES", "").split(",") if name]
if rust_sources:
    os.makedirs(gateway_home + "/helper/session", mode=0o700)
    switch = {"poll_ms": 5000, "watch": True, "sources": {}}
    for name in rust_sources:
        if name in FACTS_SOURCES:
            root = gateway_home + "/helper/session/facts/" + name
            os.makedirs(root, mode=0o700)
            switch["sources"][name] = {"facts": "native", "root": root}
        else:
            switch["sources"][name] = {"root": RUST_ROOTS[name]}
    with open(gateway_home + "/helper/session/rust-capture.json", "w") as handle:
        json.dump(switch, handle)
log = open(world + "/gateway.log", "ab")
proc = subprocess.Popen([bridge, "start", "--role", "gateway", "--home", gateway_home, "--peer-addr", "127.0.0.1:0", "--api-addr", "127.0.0.1:0", "--helper-app", app, "--helper-lab-commands"], env=env, stdout=log, stderr=log, start_new_session=True)
results = []
latencies = []
created = []
externals = []
safari_windows = []
mail_subjects = []
launched = []
EXTERNAL_BOUND = float(os.environ.get("REAL_EXTERNAL_BOUND", "45"))
OPS_TIMEOUT = 60

SCOPES = "reminders:read,reminders:write,calendar:read,calendar:write,contacts:read,contacts:write,notes:read,notes:write,system:read,safari:read,mail:read,phone:read,usage:read"
COLLECTIONS = {
    "reminders": ["reminders.list", "reminders.reminder"],
    "calendar": ["calendar.calendar", "calendar.event"],
    "contacts": ["contacts.contact"],
    "notes": ["notes.folder", "notes.note"],
    "safari": ["safari.tab", "safari.bookmark", "safari.reading_list"],
    "mail": ["mail.account", "mail.mailbox"],
    "phone": ["phone.call"],
    "usage": ["usage.app_day", "usage.device"],
}
ANCHOR = {"reminders": "reminders.list", "calendar": "calendar.calendar", "contacts": "contacts.contact", "notes": "notes.folder", "safari": "safari.history", "mail": "mail.mailbox", "phone": "phone.call", "usage": "usage.app_day"}
APP = {"notes": "Notes", "safari": "Safari", "mail": "Mail"}


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(time.strftime("%H:%M:%S ") + ("PASS " if ok else "FAIL ") + name + ((" " + str(detail)) if detail and not ok else ""), flush=True)
    return bool(ok)


def note(text):
    print("INFO " + text, flush=True)


def brief(reply):
    if not isinstance(reply, dict):
        return type(reply).__name__
    error = reply.get("error")
    if error:
        if isinstance(error, dict):
            return "error=%s/%s" % (error.get("code"), str(error.get("message", ""))[:80])
        return "error=%s" % error
    return "disposition=%s state=%s seq=%s" % (reply.get("disposition"), reply.get("state"), reply.get("commit_seq"))


def stop_gateway():
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except OSError:
        pass
    time.sleep(2)
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except OSError:
        pass


def finish(code):
    stop_gateway()
    bad = []
    for path in glob.glob(world + "/**/ab-session.log", recursive=True):
        text = open(path, errors="replace").read().lower()
        for needle in ("facts link failed", "ownership conflict"):
            if needle in text:
                bad.append("%s:%s" % (os.path.basename(os.path.dirname(path)), needle))
    logs = glob.glob(world + "/**/ab-session.log", recursive=True)
    if os.environ.get("REAL_EXPECT_SESSION_LOG") == "1":
        results.append(("session_log_present", bool(logs)))
        print(("PASS " if logs else "FAIL ") + "session_log_present", flush=True)
    results.append(("session_log_clean", not bad))
    print(("PASS " if not bad else "FAIL ") + "session_log_clean" + ((" " + ",".join(bad)) if bad else ""), flush=True)
    for name, seconds in latencies:
        print("LATENCY %s %.1fs" % (name, seconds))
    print("world %s" % world)
    print("source %s summary %d/%d" % (source, sum(1 for _, ok in results if ok), len(results)))
    sys.exit(0 if all(ok for _, ok in results) and code == 0 else 1)


deadline = time.time() + 40
runtime = None
while time.time() < deadline:
    try:
        runtime = json.load(open(gateway_home + "/runtime.json"))
        break
    except Exception:
        time.sleep(0.25)
if not check("gateway_started", runtime):
    finish(1)
token = subprocess.check_output([bridge, "token", "create", "--scope", SCOPES, "--home", gateway_home], env=env, text=True).strip()
api = "http://" + runtime["api_address"]


def op(name, body, seconds=OPS_TIMEOUT):
    request = urllib.request.Request(api + "/v1/op/" + name, data=json.dumps(body).encode(), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=seconds) as reply:
            return json.load(reply)
    except urllib.error.HTTPError as error:
        try:
            return json.load(error)
        except Exception:
            return {"error": {"code": "http_%d" % error.code}}
    except Exception as error:
        return {"error": {"code": type(error).__name__}}


def command(name, args, version=None):
    body = {"command_id": "cmd_" + secrets.token_hex(12), "op": name, "args": args, "wait_ms": 15000}
    if version is not None:
        body["expected_version"] = version
    return op(name, body)


def observed(reply):
    return isinstance(reply, dict) and not reply.get("error") and reply.get("disposition") == "locally_observed" and reply.get("commit_seq", 0) > 0


def records(collection):
    out = []
    after = None
    for _ in range(200):
        body = {"collection": collection, "limit": 500, "include_aliases": True}
        if after:
            body["after"] = after
        reply = op("records.query", body)
        if not isinstance(reply, dict) or reply.get("error"):
            break
        out.extend(reply.get("records", []))
        after = reply.get("next")
        if not after:
            break
    return out


def get(collection, record_id):
    reply = op("records.get", {"collection": collection, "id": record_id, "include_deleted": True})
    rows = reply.get("records", []) if isinstance(reply, dict) else []
    return rows[0] if len(rows) == 1 else None


def live(record):
    return record and not record.get("deleted") and not record.get("data", {}).get("recently_deleted")


def blob(record):
    return json.dumps(record.get("data", {}), sort_keys=True)


def find_tagged(collection, tag):
    for record in records(collection):
        if live(record) and tag in blob(record):
            return record
    return None


def wait_for(fn, seconds=20, gap=0.5):
    end = time.time() + seconds
    while time.time() < end:
        value = fn()
        if value:
            return value
        time.sleep(gap)
    return None


def timed(name, fn, seconds):
    started = time.time()
    value = wait_for(fn, seconds, 0.4)
    latencies.append((name, time.time() - started))
    return value


def helper_change(action, **fields):
    request = dict(fields, source=source, action=action)
    request_path = "%s/ext-%s.json" % (world, secrets.token_hex(4))
    with open(request_path, "w") as handle:
        json.dump(request, handle)
    os.makedirs(report_dir, mode=0o700, exist_ok=True)
    report_path = "%s/ab-test-ext-%s.json" % (report_dir, secrets.token_hex(6))
    try:
        subprocess.run(["open", "-n", "-W", "-g", "-a", app, "--args", "external-change", "--request", request_path, "--report", report_path], timeout=90, check=False)
        data = json.load(open(report_path))
    except Exception as error:
        data = {"state": "failed", "reason": type(error).__name__}
    finally:
        try:
            os.remove(report_path)
        except OSError:
            pass
    return data


def native_key_of(record):
    keys = []
    for alias in record.get("aliases", []) or []:
        if isinstance(alias, dict):
            keys.append(alias.get("native_key"))
    return keys


def snapshot():
    seen = {}
    for collection in COLLECTIONS[source]:
        seen[collection] = {r["id"] for r in records(collection) if live(r)}
    return seen


def gone(collection, record_id):
    record = get(collection, record_id)
    return record is None or not live(record)


def delete_checked(name, operation, collection, record_id):
    record = get(collection, record_id)
    if record is None:
        return check(name + "_target_present", False)
    version = record.get("version")
    reply = command(operation, {"id": record_id} if operation != "calendar.delete_event" else {"id": record_id, "span": "this"}, version)
    ok = check(name + "_observed", observed(reply), brief(reply))
    check(name + "_gone_in_capture", wait_for(lambda: gone(collection, record_id), 30))
    return ok


def external_delete(key):
    # Notes writes a scripted `delete` only at its next save; moving to Recently Deleted is what its window does.
    if source == "notes":
        return helper_change("trash", key=key, trash_folder_id=note_trash_folder())
    return helper_change("delete", key=key)


def external_cycle(container_title, create_title, update_title, collection):
    created_ext = helper_change("create", container_title=container_title, title=create_title)
    check("external_create_done", created_ext.get("state") == "done", created_ext.get("reason"))
    if created_ext.get("state") != "done":
        return
    key = created_ext["key"]
    externals.append(key)
    found = timed("external_create_visible", lambda: find_tagged(collection, create_title), EXTERNAL_BOUND)
    check("external_create_reaches_gateway", found, "within %ds" % EXTERNAL_BOUND)
    keys_match = bool(found) and any(key == k or k.endswith(key) or key.endswith(k or "-") for k in native_key_of(found) if k)
    note("external_create_alias_matches_native_key=%s" % keys_match)
    updated = helper_change("update", key=key, title=update_title)
    check("external_update_done", updated.get("state") == "done", updated.get("reason"))
    if updated.get("state") == "done":
        record = timed("external_update_visible", lambda: find_tagged(collection, update_title), EXTERNAL_BOUND)
        check("external_update_reaches_gateway", record, "within %ds" % EXTERNAL_BOUND)
    removed = external_delete(key)
    check("external_delete_done", removed.get("state") == "done", removed.get("reason"))
    if removed.get("state") == "done":
        externals.remove(key)
        gone_now = timed("external_delete_visible", lambda: find_tagged(collection, update_title) is None and find_tagged(collection, create_title) is None, EXTERNAL_BOUND)
        check("external_delete_reaches_gateway", gone_now, "within %ds" % EXTERNAL_BOUND)


def started_app(name):
    running = subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL).returncode == 0
    subprocess.run(["open", "-g", "-a", name], check=False)
    wait_for(lambda: subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL).returncode == 0, 20)
    time.sleep(3)
    if not running:
        launched.append(name)


def note_store_state(title):
    """Notes' own store, read-only: the live folder, Recently Deleted, or not written yet."""
    store = sqlite3.connect("file:%s/NoteStore.sqlite?mode=ro" % RUST_ROOTS["notes"].replace(" ", "%20"), uri=True)
    try:
        row = store.execute("SELECT f.ZFOLDERTYPE, o.ZMARKEDFORDELETION FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICCLOUDSYNCINGOBJECT f ON f.Z_PK = o.ZFOLDER WHERE o.ZTITLE1 = ? ORDER BY o.Z_PK DESC LIMIT 1", (title,)).fetchone()
    finally:
        store.close()
    if row is None:
        return "absent"
    return "trash" if row[0] == 1 or row[1] else "live"


def note_trash_folder():
    store = sqlite3.connect("file:%s/NoteStore.sqlite?mode=ro" % RUST_ROOTS["notes"].replace(" ", "%20"), uri=True)
    try:
        uuid = store.execute("SELECT Z_UUID FROM Z_METADATA").fetchone()[0]
        rows = store.execute("SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE ZFOLDERTYPE = 1 AND COALESCE(ZMARKEDFORDELETION, 0) = 0 AND ZPARENT IS NULL").fetchall()
    finally:
        store.close()
    if len(rows) != 1:
        raise RuntimeError("expected one Recently Deleted folder, found %d" % len(rows))
    return "x-coredata://%s/ICFolder/p%d" % (uuid, rows[0][0])


def run_notes_lifecycle(folder_title, tag):
    """A note moved to Recently Deleted leaves the live view, and moving it back returns it under the same record."""
    title = tag + " lifecycle"
    created_note = helper_change("create", container_title=folder_title, title=title)
    if not check("lifecycle_create_done", created_note.get("state") == "done", created_note.get("reason")):
        return
    key = created_note["key"]
    externals.append(key)
    found = timed("lifecycle_create_visible", lambda: find_tagged("notes.note", title), EXTERNAL_BOUND)
    if not check("lifecycle_create_reaches_gateway", found):
        return
    for attempt in ("a", "b"):
        trashed = helper_change("trash", key=key, trash_folder_id=note_trash_folder())
        check("lifecycle_trash_done_" + attempt, trashed.get("state") == "done", trashed.get("reason"))
        check("lifecycle_trash_in_store_" + attempt, timed("lifecycle_trash_store_" + attempt, lambda: note_store_state(title) == "trash", EXTERNAL_BOUND))
        check("lifecycle_trash_reaches_gateway_" + attempt, timed("lifecycle_trash_visible_" + attempt, lambda: gone("notes.note", found["id"]), EXTERNAL_BOUND))
        restored = helper_change("restore", key=key, container_title=folder_title)
        check("lifecycle_restore_done_" + attempt, restored.get("state") == "done", restored.get("reason"))
        back = timed("lifecycle_restore_visible_" + attempt, lambda: (lambda r: r if live(r) else None)(get("notes.note", found["id"])), EXTERNAL_BOUND)
        check("lifecycle_restore_reaches_gateway_" + attempt, back)
        if back:
            check("lifecycle_restore_has_body_" + attempt, wait_for(lambda: "external" in blob(get("notes.note", found["id"]) or {}), EXTERNAL_BOUND))
    pressed = helper_change("window-delete", key=key)
    check("window_delete_done", pressed.get("state") == "done", pressed.get("reason"))
    if pressed.get("state") == "done":
        check("window_delete_in_store", timed("window_delete_store", lambda: note_store_state(title) == "trash", EXTERNAL_BOUND))
        check("window_delete_reaches_gateway", timed("window_delete_visible", lambda: gone("notes.note", found["id"]), EXTERNAL_BOUND))
        restored = helper_change("restore", key=key, container_title=folder_title)
        check("window_restore_reaches_gateway", restored.get("state") == "done" and timed("window_restore_visible", lambda: (lambda r: r if live(r) else None)(get("notes.note", found["id"])), EXTERNAL_BOUND), restored.get("reason"))
    scripted = helper_change("delete", key=key)
    if scripted.get("state") == "done":
        externals.remove(key)
    # Scripted delete: measured, not asserted. Notes writes it at its next save, and the gateway follows the store.
    timed("scripted_delete_store_commit", lambda: note_store_state(title) == "trash", float(os.environ.get("REAL_SCRIPTED_BOUND", "30")))
    note("scripted delete store_state=%s" % note_store_state(title))
    # Any later save writes it: this note is what makes Notes save here.
    nudge = helper_change("create", container_title=folder_title, title=tag + " nudge")
    if nudge.get("state") == "done":
        externals.append(nudge["key"])
    check("scripted_delete_written_by_next_save", wait_for(lambda: note_store_state(title) == "trash", EXTERNAL_BOUND))
    check("scripted_delete_reaches_gateway_once_written", timed("scripted_delete_visible", lambda: gone("notes.note", found["id"]), EXTERNAL_BOUND))


def tab_with(token):
    return next((r for r in records("safari.tab") if live(r) and token in blob(r)), None)


def history_with(token):
    return next((r for r in records("safari.history") if live(r) and token in blob(r)), None)


def run_safari(tag):
    started_app("Safari")
    token = "abtest" + secrets.token_hex(4)
    first = "https://example.com/?ab-test=%s-a" % token
    second = "https://example.com/?ab-test=%s-b" % token
    made = helper_change("create", url=first)
    if not check("external_create_done", made.get("state") == "done", made.get("reason")):
        return
    key = made["key"]
    safari_windows.append((key, token))
    check("tab_create_reaches_gateway", timed("tab_create_visible", lambda: tab_with(token + "-a"), EXTERNAL_BOUND))
    check("history_visit_reaches_gateway", timed("history_create_visible", lambda: history_with(token + "-a"), float(os.environ.get("REAL_HISTORY_BOUND", "120"))))
    rounds = int(os.environ.get("REAL_TAB_ROUNDS", "3"))
    for index in range(rounds):
        marker = "%s-n%d" % (token, index)
        moved = helper_change("update", key=key, marker=token, url="https://example.com/?ab-test=%s" % marker)
        check("tab_update_done_%d" % index, moved.get("state") == "done", moved.get("reason"))
        check("tab_update_reaches_gateway_%d" % index, timed("tab_update_%d" % index, lambda: tab_with(marker), EXTERNAL_BOUND))
        check("tab_old_url_replaced_%d" % index, tab_with(token + "-a") is None)
        time.sleep(float(os.environ.get("REAL_TAB_GAP", "30")))
    moved = helper_change("update", key=key, marker=token, url=second)
    check("external_update_done", moved.get("state") == "done", moved.get("reason"))
    check("tab_update_reaches_gateway", timed("tab_update_visible", lambda: tab_with(token + "-b"), EXTERNAL_BOUND))
    closed = helper_change("delete", key=key, marker=token)
    if check("external_delete_done", closed.get("state") == "done", closed.get("reason")):
        safari_windows.remove((key, token))
        check("tab_close_reaches_gateway", timed("tab_close_visible", lambda: tab_with(token) is None, EXTERNAL_BOUND))
    note("safari history visits stay in Safari, which offers no scripted removal: https://example.com/?ab-test=%s-*" % token)


def mail_records(subject):
    return [r for r in records("mail.message") if subject in blob(r)]


def mail_in(subject, predicate):
    return next((r for r in mail_records(subject) if live(r) and predicate(r)), None)


def run_mail(tag):
    started_app("Mail")
    subject = tag + " draft"
    made = helper_change("create", title=subject)
    if not check("external_create_done", made.get("state") == "done", made.get("reason")):
        return
    mail_subjects.append(subject)
    bound = float(os.environ.get("REAL_MAIL_BOUND", "120"))
    record = timed("draft_create_visible", lambda: mail_in(subject, lambda r: True), bound)
    if not check("draft_reaches_gateway", record):
        return
    flagged = helper_change("update", title=subject)
    if check("external_update_done", flagged.get("state") == "done", flagged.get("reason")):
        check("flag_reaches_gateway", timed("flag_visible", lambda: mail_in(subject, lambda r: r.get("data", {}).get("flagged")), bound))
    removed = helper_change("delete", title=subject)
    if check("external_delete_done", removed.get("state") == "done", removed.get("reason")):
        mail_subjects.remove(subject)
        # Mail deletes by moving to Deleted Messages, as a new row: the draft's record leaves and the new one is in another mailbox.
        check("delete_reaches_gateway", timed("delete_visible", lambda: gone("mail.message", record["id"]) or mail_in(subject, lambda r: r.get("data", {}).get("mailbox_ids") != record.get("data", {}).get("mailbox_ids")), bound))


def run_usage(tag):
    bundle = os.environ.get("REAL_USAGE_BUNDLE", "com.apple.calculator")
    app_name = os.environ.get("REAL_USAGE_APP", "Calculator")
    today = time.strftime("%Y-%m-%d")

    def seconds():
        total = 0
        for row in records("usage.app_day"):
            data = row.get("data", {})
            if live(row) and data.get("bundle_id") == bundle and str(data.get("day")).startswith(today):
                total += data.get("seconds_estimated", 0) or 0
        return total

    before_seconds = seconds()
    note("usage %s today before=%s" % (bundle, before_seconds))
    running = subprocess.run(["pgrep", "-x", app_name], stdout=subprocess.DEVNULL).returncode == 0
    if not running:
        launched.append(app_name)
    subprocess.run(["open", "-a", app_name], check=False)
    time.sleep(float(os.environ.get("REAL_USAGE_FOCUS", "45")))
    subprocess.run(["open", "-a", "Finder"], check=False)
    grown = timed("usage_visible", lambda: seconds() > before_seconds, float(os.environ.get("REAL_USAGE_BOUND", "300")))
    check("usage_focus_reaches_gateway", grown)
    note("usage %s today after=%s" % (bundle, seconds()))


def run_phone(tag):
    store = sqlite3.connect("file:%s/CallHistory.storedata?mode=ro" % RUST_ROOTS["phone"].replace(" ", "%20"), uri=True)
    in_store = store.execute("SELECT COUNT(*), MAX(ZDATE) FROM ZCALLRECORD").fetchone()
    store.close()
    calls = [r for r in records("phone.call") if live(r)]
    note("phone store=%d gateway=%d newest_store_age_days=%.1f" % (in_store[0], len(calls), (time.time() - 978307200 - (in_store[1] or 0)) / 86400))
    check("gateway_matches_call_history", len(calls) == in_store[0], "%d vs %d" % (len(calls), in_store[0]))
    note("no call placed: a new call reaching the gateway is NOT proven here")


def run_reminders(tag):
    reply = command("reminders.create_list", {"title": tag})
    check("create_list_observed", observed(reply), brief(reply))
    owned = wait_for(lambda: find_tagged("reminders.list", tag))
    check("created_list_visible", owned)
    if not owned:
        return
    created.append(("reminders.delete_list", "reminders.list", owned["id"]))
    title = tag + " item"
    reply = command("reminders.create", {"list_id": owned["id"], "title": title})
    check("create_observed", observed(reply), brief(reply))
    item = wait_for(lambda: find_tagged("reminders.reminder", title))
    check("created_visible", item)
    if item:
        created.append(("reminders.delete", "reminders.reminder", item["id"]))
        for operation in ["update", "complete", "uncomplete"]:
            args = {"id": item["id"]}
            if operation == "update":
                args["notes"] = "[ab-test] revised"
            reply = command("reminders." + operation, args, item["version"])
            check(operation + "_observed", observed(reply), brief(reply))
            fresh = get("reminders.reminder", item["id"])
            if fresh:
                item = fresh
        check("update_visible_in_capture", item.get("data", {}).get("notes") == "[ab-test] revised")
        stale = command("reminders.update", {"id": item["id"], "notes": "[ab-test] stale"}, 1)
        check("stale_version_rejected", isinstance(stale, dict) and stale.get("error"))
        delete_checked("delete", "reminders.delete", "reminders.reminder", item["id"])
    external_cycle(tag, tag + " ext", tag + " ext revised", "reminders.reminder")
    delete_checked("delete_list", "reminders.delete_list", "reminders.list", owned["id"])


def calendar_enumeration_proof():
    """A pending Reminders hint must not hold up a Calendar enumeration: the Rust loop pulls Calendar first, so it would wait for itself."""
    report_path = "%s/ab-test-cal-%s.json" % (report_dir, secrets.token_hex(6))
    try:
        subprocess.run(["open", "-n", "-W", "-g", "-a", app, "--args", "calendar-facts-proof", "--report", report_path], timeout=60, check=False)
        data = json.load(open(report_path))
    except Exception as error:
        data = {"state": "failed", "reason": type(error).__name__}
    finally:
        try:
            os.remove(report_path)
        except OSError:
            pass
    check("calendar_enumerates_with_reminder_hint_pending", data.get("state") == "done", data)


def calendar_update_rounds(tag):
    """One outside event updated again and again, at gaps that sweep the pull interval. Every update has to arrive."""
    rounds = int(os.environ.get("REAL_CALENDAR_ROUNDS", "0"))
    if not rounds:
        return
    title = tag + " rounds"
    made = helper_change("create", container_title="Bridge Test", title=title)
    if not check("rounds_create_done", made.get("state") == "done", made.get("reason")):
        return
    key = made["key"]
    externals.append(key)
    check("rounds_create_reaches_gateway", timed("rounds_create", lambda: find_tagged("calendar.event", title), EXTERNAL_BOUND))
    seen = []
    missed = 0
    for index in range(rounds):
        marker = "%s %d" % (title, index)
        started = time.time()
        moved = helper_change("update", key=key, title=marker)
        if moved.get("state") != "done":
            check("rounds_update_done_%d" % index, False, moved.get("reason"))
            continue
        arrived = wait_for(lambda: find_tagged("calendar.event", marker), EXTERNAL_BOUND, 0.4)
        seconds = time.time() - started
        seen.append(seconds)
        latencies.append(("rounds_update_%d" % index, seconds))
        missed += 0 if arrived else 1
        time.sleep(random.uniform(0, float(os.environ.get("REAL_CALENDAR_GAP", "18"))))
    seen.sort()
    note("calendar rounds=%d missed=%d median=%.1fs max=%.1fs" % (len(seen), missed, seen[len(seen) // 2] if seen else 0, seen[-1] if seen else 0))
    check("every_calendar_update_arrived", missed == 0, "%d missed" % missed)
    removed = helper_change("delete", key=key)
    if removed.get("state") == "done":
        externals.remove(key)


def run_calendar(tag):
    calendar_enumeration_proof()
    fixtures = [r for r in records("calendar.calendar") if live(r) and r.get("data", {}).get("title") == "Bridge Test"]
    note("calendar_fixture_candidates=%d fields=%s" % (len(fixtures), sorted(fixtures[0].get("data", {}).keys()) if fixtures else []))
    if not check("calendar_fixture_present", fixtures):
        return
    calendar_id = fixtures[0]["id"]
    title = tag + " event"
    start = time.strftime("%Y-%m-%dT15:00:00Z", time.gmtime(time.time() + 40 * 86400))
    end = time.strftime("%Y-%m-%dT16:00:00Z", time.gmtime(time.time() + 40 * 86400))
    reply = command("calendar.create_event", {"calendar_id": calendar_id, "title": title, "start": start, "end": end, "time_zone": "UTC"})
    check("create_observed", observed(reply), brief(reply))
    event = wait_for(lambda: find_tagged("calendar.event", title))
    check("created_visible", event)
    if event:
        created.append(("calendar.delete_event", "calendar.event", event["id"]))
        reply = command("calendar.update_event", {"id": event["id"], "span": "this", "title": title + " revised", "location": "[ab-test] room"}, event["version"])
        check("update_observed", observed(reply), brief(reply))
        revised = wait_for(lambda: find_tagged("calendar.event", title + " revised"))
        check("update_visible_in_capture", revised)
        stale = command("calendar.update_event", {"id": event["id"], "span": "this", "title": title + " stale"}, 1)
        check("stale_version_rejected", isinstance(stale, dict) and stale.get("error"))
        delete_checked("delete", "calendar.delete_event", "calendar.event", event["id"])
    external_cycle("Bridge Test", tag + " ext", tag + " ext revised", "calendar.event")
    calendar_update_rounds(tag)


def run_contacts(tag):
    data = {"given_name": tag, "family_name": "Testfamily", "organization": "Bridge Test", "job_title": "Testjob", "phones": [{"label": "mobile", "value": "+15555550123"}], "emails": [{"label": "home", "value": "bridge-test@example.com"}]}
    reply = command("contacts.create", data)
    check("create_observed", observed(reply), brief(reply))
    contact = wait_for(lambda: find_tagged("contacts.contact", tag))
    check("created_visible", contact)
    if contact:
        created.append(("contacts.delete", "contacts.contact", contact["id"]))
        reply = command("contacts.update", {"id": contact["id"], "job_title": "Updated Testjob"}, contact["version"])
        check("update_observed", observed(reply), brief(reply))
        updated = wait_for(lambda: (lambda r: r if r and "Updated Testjob" in blob(r) else None)(find_tagged("contacts.contact", tag)))
        check("update_visible_in_capture", updated)
        stale = command("contacts.update", {"id": contact["id"], "job_title": "Stale Testjob"}, 1)
        check("stale_version_rejected", isinstance(stale, dict) and stale.get("error"))
        delete_checked("delete", "contacts.delete", "contacts.contact", contact["id"])
    external_cycle("", tag + " ext", tag + " ext revised", "contacts.contact")


def run_notes(tag):
    folders = [r for r in records("notes.folder") if live(r)]
    chosen = None
    for wanted in ("Bridge Test", "Notes"):
        for folder in folders:
            if folder.get("data", {}).get("title") == wanted:
                chosen = chosen or (folder, wanted)
    note("notes_folders=%d chosen=%s" % (len(folders), chosen[1] if chosen else None))
    if not check("notes_folder_present", chosen):
        return
    folder, folder_title = chosen
    title = tag + " note"
    reply = command("notes.create", {"folder_id": folder["id"], "title": title, "plain_text": "[ab-test] body"})
    check("create_observed", observed(reply), brief(reply))
    item = wait_for(lambda: find_tagged("notes.note", title))
    check("created_visible", item)
    if item:
        created.append(("notes.delete", "notes.note", item["id"]))
        reply = command("notes.update", {"id": item["id"], "plain_text": "[ab-test] appended", "mode": "append"}, item["version"])
        check("update_observed", observed(reply), brief(reply))
        updated = wait_for(lambda: (lambda r: r if r and "appended" in blob(r) else None)(find_tagged("notes.note", title)))
        check("update_visible_in_capture", updated)
        stale = command("notes.update", {"id": item["id"], "plain_text": "[ab-test] stale", "mode": "append"}, 1)
        check("stale_version_rejected", isinstance(stale, dict) and stale.get("error"))
        delete_checked("delete", "notes.delete", "notes.note", item["id"])
    external_cycle(folder_title, tag + " ext", tag + " ext revised", "notes.note")
    run_notes_lifecycle(folder_title, tag)


def cleanup_created():
    for operation, collection, record_id in reversed(created):
        record = get(collection, record_id)
        if record and live(record):
            reply = command(operation, {"id": record_id} if operation != "calendar.delete_event" else {"id": record_id, "span": "this"}, record.get("version"))
            note("cleanup %s by id: %s" % (operation, brief(reply)))
            wait_for(lambda: gone(collection, record_id), 30)
    if source == "notes":
        for record in records("notes.note"):
            if live(record) and tag in blob(record):
                reply = command("notes.delete", {"id": record["id"]}, record.get("version"))
                note("cleanup late note by id: %s" % brief(reply))
    for key in list(externals):
        removed = external_delete(key)
        note("cleanup external by key: %s" % removed.get("state"))
    for key, marker in list(safari_windows):
        note("cleanup safari tab: %s" % helper_change("delete", key=key, marker=marker).get("state"))
    for subject in list(mail_subjects):
        note("cleanup mail draft: %s" % helper_change("delete", title=subject).get("state"))
    for name in launched:
        subprocess.run(["pkill", "-TERM", "-x", name], check=False, timeout=30)


ready = wait_for(lambda: any(r.get("data", {}).get("source") == source and r.get("data", {}).get("state") == "ready" for r in records("system.source")), 240, 1)
check("%s_source_ready" % source, ready)
if not ready:
    finish(1)
check("capture_has_%s" % ANCHOR[source], wait_for(lambda: len(records(ANCHOR[source])) > 0, 300, 2))
def settled():
    """The first capture arrives in batches. Commands and baselines wait until the counts stop growing."""
    last = None
    quiet_since = time.time()
    deadline = time.time() + float(os.environ.get("REAL_SETTLE_BOUND", "300"))
    while time.time() < deadline:
        counts = {collection: len(records(collection)) for collection in COLLECTIONS[source]}
        if counts != last:
            last = counts
            quiet_since = time.time()
        elif time.time() - quiet_since >= 15:
            return True
        time.sleep(2)
    return False


check("first_capture_settled", settled())
time.sleep(float(os.environ.get("REAL_SETTLE_EXTRA", "0")))
before = snapshot()
note("before " + ",".join("%s=%d" % (k, len(v)) for k, v in before.items()))
tag = "[ab-test] real-%s %s" % (source, secrets.token_hex(3))
try:
    if source == "notes":
        started_app("Notes")
        started_app("System Events")
    {"reminders": run_reminders, "calendar": run_calendar, "contacts": run_contacts, "notes": run_notes, "safari": run_safari, "mail": run_mail, "usage": run_usage, "phone": run_phone}[source](tag)
except Exception as error:
    check("driver_exception", False, type(error).__name__ + ":" + str(error)[:120])
finally:
    cleanup_created()
time.sleep(3)
after = snapshot()
for collection, ids in before.items():
    missing = ids - after.get(collection, set())
    check("preexisting_%s_untouched" % collection, not missing, "%d missing" % len(missing))
leftover = [r for collection in COLLECTIONS[source] for r in records(collection) if live(r) and tag in blob(r)]
check("no_leftover_test_records", not leftover, "%d left" % len(leftover))
finish(0)
