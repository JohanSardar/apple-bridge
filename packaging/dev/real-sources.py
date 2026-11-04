import glob, json, os, secrets, signal, subprocess, sys, time, urllib.error, urllib.request

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
log = open(world + "/gateway.log", "ab")
proc = subprocess.Popen([bridge, "start", "--role", "gateway", "--home", gateway_home, "--peer-addr", "127.0.0.1:0", "--api-addr", "127.0.0.1:0", "--helper-app", app, "--helper-lab-commands"], env=env, stdout=log, stderr=log, start_new_session=True)
results = []
latencies = []
created = []
externals = []
EXTERNAL_BOUND = 45.0
OPS_TIMEOUT = 60

SCOPES = "reminders:read,reminders:write,calendar:read,calendar:write,contacts:read,contacts:write,notes:read,notes:write,system:read"
COLLECTIONS = {
    "reminders": ["reminders.list", "reminders.reminder"],
    "calendar": ["calendar.calendar", "calendar.event"],
    "contacts": ["contacts.contact"],
    "notes": ["notes.folder", "notes.note"],
}
ANCHOR = {"reminders": "reminders.list", "calendar": "calendar.calendar", "contacts": "contacts.contact", "notes": "notes.folder"}


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + ((" " + str(detail)) if detail and not ok else ""), flush=True)
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
    except ProcessLookupError:
        pass
    time.sleep(2)
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
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
    removed = helper_change("delete", key=key)
    check("external_delete_done", removed.get("state") == "done", removed.get("reason"))
    if removed.get("state") == "done":
        externals.remove(key)
        gone_now = timed("external_delete_visible", lambda: find_tagged(collection, update_title) is None and find_tagged(collection, create_title) is None, EXTERNAL_BOUND)
        check("external_delete_reaches_gateway", gone_now, "within %ds" % EXTERNAL_BOUND)


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


def run_calendar(tag):
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


def cleanup_created():
    for operation, collection, record_id in reversed(created):
        record = get(collection, record_id)
        if record and live(record):
            reply = command(operation, {"id": record_id} if operation != "calendar.delete_event" else {"id": record_id, "span": "this"}, record.get("version"))
            note("cleanup %s by id: %s" % (operation, brief(reply)))
            wait_for(lambda: gone(collection, record_id), 30)
    for key in list(externals):
        removed = helper_change("delete", key=key)
        note("cleanup external by key: %s" % removed.get("state"))


ready = wait_for(lambda: any(r.get("data", {}).get("source") == source and r.get("data", {}).get("state") == "ready" for r in records("system.source")), 240, 1)
check("%s_source_ready" % source, ready)
if not ready:
    finish(1)
check("capture_has_%s" % ANCHOR[source], wait_for(lambda: len(records(ANCHOR[source])) > 0, 300, 2))
before = snapshot()
note("before " + ",".join("%s=%d" % (k, len(v)) for k, v in before.items()))
tag = "[ab-test] real-%s %s" % (source, secrets.token_hex(3))
try:
    {"reminders": run_reminders, "calendar": run_calendar, "contacts": run_contacts, "notes": run_notes}[source](tag)
except Exception as error:
    check("driver_exception", False, type(error).__name__ + ":" + str(error)[:120])
finally:
    cleanup_created()
time.sleep(3)
after = snapshot()
for collection, ids in before.items():
    missing = ids - after.get(collection, set())
    check("preexisting_%s_untouched" % collection, not missing, "%d missing" % len(missing))
leftover = [r for collection in COLLECTIONS[source] for r in records(collection) if live(r) and "[ab-test] real-" in blob(r)]
check("no_leftover_test_records", not leftover, "%d left" % len(leftover))
finish(0)
