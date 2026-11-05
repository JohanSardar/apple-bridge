#!/usr/bin/env python3
"""Runs real commands through a gateway on a private home, with negative tests for the dangerous rules.

usage: real-commands.py --home HOME --bridge BRIDGE --app APP --ledger FILE SOURCE [SOURCE...]

HOME is a gateway started by copy-proof.py --hold. Everything created is tagged [ab-test] and recorded by id in
the ledger; cleanup removes only those ids. Prints pass or fail lines and field names, never personal values.
Mail is only ever sent to REAL_SELF_ADDRESS. Stops at any permission prompt for the Helper.
"""
import argparse, base64, hashlib, json, os, plistlib, secrets, sqlite3, subprocess, sys, tempfile, time, urllib.error, urllib.request

parser = argparse.ArgumentParser()
parser.add_argument("--home", required=True)
parser.add_argument("--bridge", required=True)
parser.add_argument("--app", required=True)
parser.add_argument("--ledger", required=True)
parser.add_argument("sources", nargs="+")
options = parser.parse_args()
HOME = os.path.expanduser("~")
BRIDGE_TEST = HOME + "/Library/Mobile Documents/com~apple~CloudDocs/Bridge Test"
MAIL_INDEX = HOME + "/Library/Mail/V10/MailData/Envelope Index"
REPORTS = HOME + "/Library/Application Support/AppleBridgeDev"
SCOPES = ",".join(s + ":read," + s + ":write" for s in ("mail", "files", "mac", "focus", "shortcuts", "pdf", "capture", "speech", "maps", "calendar", "print")) + ",system:read"
env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE="file")
runtime = json.load(open(options.home + "/runtime.json"))
token = subprocess.check_output([options.bridge, "token", "create", "--scope", SCOPES, "--home", options.home], env=env, text=True).strip()
api = "http://" + runtime["api_address"]
since = time.strftime("%Y-%m-%d %H:%M:%S")
tag = "[ab-test] cmd-" + secrets.token_hex(3)
results = []


def ledger(kind, value):
    with open(options.ledger, "a") as handle:
        handle.write(json.dumps({"at": time.time(), "kind": kind, "value": value, "tag": tag}) + "\n")


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(time.strftime("%H:%M:%S ") + ("PASS " if ok else "FAIL ") + name + ((" " + str(detail)) if detail and not ok else ""), flush=True)
    return bool(ok)


def prompted():
    out = subprocess.run(["log", "show", "--start", since, "--style", "compact", "--predicate", 'process == "tccd" AND eventMessage CONTAINS "AUTHREQ_PROMPTING"'], capture_output=True, text=True, timeout=120).stdout
    return [line for line in out.splitlines() if "applebridge" in line.lower()]


def op(name, body, seconds=120):
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


def command(name, args, version=None, seconds=120):
    body = {"command_id": "cmd_" + secrets.token_hex(12), "op": name, "args": args, "wait_ms": 15000}
    if version is not None:
        body["expected_version"] = version
    reply = op(name, body, seconds)
    end = time.time() + seconds
    while isinstance(reply, dict) and not reply.get("error") and not reply.get("disposition") and time.time() < end:
        time.sleep(1)
        status = op("commands.status", {"command_id": body["command_id"]})
        rows = status.get("commands") if isinstance(status, dict) else None
        reply = rows[0] if rows else status
    return reply


def query(name, args):
    return op(name, {"op": name, "args": args})


def brief(reply):
    if not isinstance(reply, dict):
        return type(reply).__name__
    error = reply.get("error")
    if error:
        return "error=%s" % (error.get("code") if isinstance(error, dict) else error)
    return "disposition=%s state=%s" % (reply.get("disposition"), reply.get("state"))


def observed(reply):
    return isinstance(reply, dict) and not reply.get("error") and reply.get("disposition") == "locally_observed"


def refused(reply, *codes):
    if not isinstance(reply, dict):
        return False
    error = reply.get("error")
    code = error.get("code") if isinstance(error, dict) else error
    return code in codes and reply.get("disposition") in (None, "rejected_before_dispatch", "failed_no_effect")


def records(collection, limit=500):
    reply = op("records.query", {"collection": collection, "limit": limit, "include_aliases": True})
    return reply.get("records", []) if isinstance(reply, dict) else []


def wait_for(fn, seconds=60, gap=0.5):
    end = time.time() + seconds
    while time.time() < end:
        value = fn()
        if value:
            return value
        time.sleep(gap)
    return None


def store_rows(subject):
    store = sqlite3.connect("file:%s?mode=ro" % MAIL_INDEX.replace(" ", "%20"), uri=True, timeout=3)
    try:
        return [(row, url.rsplit("/", 1)[-1]) for row, url in store.execute("select x.ROWID,m.url from messages x join subjects s on s.ROWID=x.subject join mailboxes m on m.ROWID=x.mailbox where s.subject=? and x.deleted=0", (subject,))]
    finally:
        store.close()


def mail_record(subject, predicate=lambda r: True):
    for record in records_search(subject):
        if not record.get("deleted") and record.get("data", {}).get("subject") == subject and predicate(record):
            return record
    return None


def records_search(subject):
    reply = op("search", {"text": subject, "collection": "mail.message", "limit": 20})
    rows = reply.get("records") if isinstance(reply, dict) else None
    return rows or []


def mail_was_running():
    return subprocess.run(["pgrep", "-x", "Mail"], stdout=subprocess.DEVNULL).returncode == 0


def run_mail():
    if os.environ.get("REAL_MUTATED") == "1":
        print("INFO mail: mutated build, drafts only")
    was = mail_was_running()
    subject = tag + " draft"
    draft = {"to": ["ab-test@example.invalid"], "subject": subject, "body": "synthetic body"}
    for name, args in [
        ("no_recipient", {"subject": subject}),
        ("empty_recipient", {"to": [], "subject": subject}),
        ("no_subject", {"to": ["ab-test@example.invalid"]}),
        ("comma_recipient", {"to": ["ab-test@example.invalid, other@example.invalid"], "subject": subject}),
        ("newline_recipient", {"to": ["ab-test@example.invalid\nbcc: other@example.invalid"], "subject": subject}),
        ("display_name", {"to": ["Name <ab-test@example.invalid>"], "subject": subject}),
        ("bad_cc", {"to": ["ab-test@example.invalid"], "cc": ["nope"], "subject": subject}),
        ("bcc_field", {"to": ["ab-test@example.invalid"], "bcc": ["other@example.invalid"], "subject": subject}),
    ]:
        for op_name in ("mail.create_draft",) if os.environ.get("REAL_MUTATED") == "1" else ("mail.create_draft", "mail.send"):
            reply = command(op_name, args)
            check("mail_refused_%s_%s" % (name, op_name.split(".")[1]), refused(reply, "invalid"), brief(reply))
    for name, args in [("attachment", dict(draft, attachments=["/etc/hosts"])), ("reply", dict(draft, reply_to="1")), ("forward", dict(draft, forward_of="1"))]:
        reply = command("mail.send", args)
        check("mail_unsupported_%s" % name, refused(reply, "unsupported"), brief(reply))
    check("mail_refusals_left_no_row", not store_rows(subject))
    reply = command("mail.create_draft", draft)
    if not check("mail_create_draft_observed", observed(reply), brief(reply)):
        return
    rows = wait_for(lambda: store_rows(subject), 30)
    if not check("mail_draft_in_mails_store", rows and rows[0][1] == "Drafts", rows):
        return
    for row, _ in rows:
        ledger("mail", row)
    record = wait_for(lambda: mail_record(subject), 60)
    if not check("mail_draft_reaches_gateway", record):
        return
    reply = command("mail.create_draft", draft)
    check("mail_second_copy_refused", refused(reply, "conflict"), brief(reply))
    check("mail_second_copy_no_row", len(store_rows(subject)) == 1)
    reply = command("mail.set_flag", {"id": record["id"], "flagged": True}, record["version"])
    check("mail_set_flag_observed", observed(reply), brief(reply))
    flagged = wait_for(lambda: mail_record(subject, lambda r: r["data"].get("flagged")), 60)
    check("mail_flag_reaches_gateway", flagged)
    current = flagged or mail_record(subject)
    reply = command("mail.mark_read", {"id": current["id"], "read": False}, current["version"])
    check("mail_mark_unread_observed", observed(reply), brief(reply))
    current = wait_for(lambda: mail_record(subject, lambda r: r["data"].get("read") is False), 60)
    check("mail_read_state_reaches_gateway", current)
    current = current or mail_record(subject)
    stale = command("mail.set_flag", {"id": current["id"], "flagged": False}, current["version"] + 99)
    check("mail_stale_version_refused", refused(stale, "conflict"), brief(stale))
    reply = command("mail.delete", {"id": current["id"]}, current["version"])
    check("mail_delete_observed", observed(reply), brief(reply))
    moved = wait_for(lambda: [row for row in store_rows(subject) if "Deleted" in row[1]], 30)
    check("mail_delete_moved_to_trash", moved, store_rows(subject))
    for row, _ in moved or []:
        ledger("mail", row)
    check("mail_delete_tombstoned_old_record", wait_for(lambda: mail_record(subject, lambda r: r["id"] == current["id"]) is None, 60))
    address = os.environ.get("REAL_SELF_ADDRESS", "")
    if address:
        sent_subject = tag + " send"
        reply = command("mail.send", {"to": [address], "subject": sent_subject, "body": "Apple Bridge test message to yourself."})
        check("mail_send_self_observed", observed(reply), brief(reply))
        rows = wait_for(lambda: store_rows(sent_subject), 60)
        check("mail_send_self_in_store", rows, rows)
        for row, _ in rows or []:
            ledger("mail", row)
        both = wait_for(lambda: (lambda found: found if len(found) == 2 else None)(store_rows(sent_subject)), 120, 2)
        if check("mail_self_send_has_two_copies", both, store_rows(sent_subject)):
            for index in range(2):
                trash = next((r["id"] for r in records("mail.mailbox") if "Deleted" in r["data"].get("name", "")), "")
                copy = wait_for(lambda: next((r for r in records_search(sent_subject) if not r.get("deleted") and r["data"].get("subject") == sent_subject and trash not in json.dumps(r["data"].get("mailbox_ids"))), None), 60)
                if not check("mail_delete_copy_%d_found" % index, copy):
                    break
                reply = command("mail.delete", {"id": copy["id"]}, copy["version"])
                check("mail_delete_copy_%d_observed" % index, observed(reply), brief(reply))
            check("mail_both_copies_in_trash", wait_for(lambda: len([row for row in store_rows(sent_subject) if "Deleted" in row[1]]) == 2, 30), store_rows(sent_subject))
    if not was:
        subprocess.run(["pkill", "-TERM", "-x", "Mail"])


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def encoded(text):
    return base64.b64encode(text.encode()).decode()


def run_files():
    root = os.path.realpath(BRIDGE_TEST)
    folder = tag + " folder"
    path = os.path.join(root, folder)
    os.makedirs(path)
    ledger("file", path)
    outside = tempfile.mkdtemp(prefix="ab-test-outside-")
    secret = os.path.join(outside, "secret.txt")
    open(secret, "w").write("outside")
    os.symlink(outside, os.path.join(path, "link"))
    os.symlink(secret, os.path.join(path, "file-link"))
    named = lambda name, content="x": {"root": root, "parent": folder, "name": name, "content_base64": encoded(content), "digest": digest(content)}
    wrote_outside = lambda: sorted(os.listdir(outside)) != ["secret.txt"] or open(secret).read() != "outside"
    for name, op_name, args, codes in [
        ("root_not_configured", "files.create", dict(named("a.txt"), root=outside, parent=""), ("invalid",)),
        ("root_home", "files.create", dict(named("a.txt"), root=HOME, parent=""), ("invalid",)),
        ("parent_dotdot", "files.create", dict(named("a.txt"), parent="../.."), ("invalid",)),
        ("parent_absolute", "files.create", dict(named("a.txt"), parent=outside), ("invalid",)),
        ("parent_symlink", "files.create", dict(named("a.txt"), parent=folder + "/link"), ("invalid",)),
        ("name_slash", "files.create", dict(named("a.txt"), name="../a.txt"), ("invalid",)),
        ("name_dotdot", "files.create", dict(named("a.txt"), name=".."), ("invalid",)),
        ("name_over_symlink", "files.write", named("file-link"), ("invalid",)),
        ("bad_digest", "files.create", dict(named("a.txt"), digest="0" * 64), ("invalid",)),
        ("trash_symlink", "files.trash", {"root": root, "path": folder + "/link/secret.txt"}, ("invalid",)),
        ("trash_dotdot", "files.trash", {"root": root, "path": "../../etc"}, ("invalid",)),
        ("move_from_symlink", "files.move", {"root": root, "path": folder + "/link/secret.txt", "parent": folder, "name": "stolen.txt"}, ("invalid",)),
        ("tags_on_symlink", "files.set_tags", {"root": root, "path": folder + "/file-link", "tags": ["x"]}, ("invalid",)),
        ("evict_symlink", "files.evict", {"root": root, "path": folder + "/link/secret.txt"}, ("invalid",)),
    ]:
        reply = command(op_name, args)
        check("files_refused_%s" % name, refused(reply, *codes), brief(reply))
    check("files_nothing_changed_outside", not wrote_outside())
    for name, args in [("outside", {"root": root, "path": "../../../../etc/hosts"}), ("symlink", {"root": root, "path": folder + "/link/secret.txt"}), ("other_root", {"root": outside, "path": "secret.txt"})]:
        reply = query("files.read", args)
        check("files_read_refused_%s" % name, isinstance(reply, dict) and reply.get("error"), brief(reply))
    file = os.path.join(path, tag + " a.txt")
    reply = command("files.create", named(tag + " a.txt", "first " + tag))
    check("files_create_observed", observed(reply), brief(reply))
    check("files_create_on_disk", os.path.exists(file) and open(file).read() == "first " + tag)
    reply = command("files.create", named(tag + " a.txt", "again"))
    check("files_create_never_overwrites", open(file).read() == "first " + tag, brief(reply))
    reply = command("files.write", named(tag + " a.txt", "second " + tag))
    check("files_write_observed", observed(reply), brief(reply))
    check("files_write_on_disk", open(file).read() == "second " + tag)
    item = wait_for(lambda: next((r for r in records("files.item", 1000) if r.get("data", {}).get("path", "").endswith(tag + " a.txt") and not r.get("deleted")), None), 60)
    check("files_item_reaches_gateway", item)
    reply = command("files.set_tags", {"root": root, "path": folder + "/" + tag + " a.txt", "tags": ["Bridge Test"]})
    check("files_set_tags_observed", observed(reply), brief(reply))
    reply = query("files.read", {"root": root, "path": folder + "/" + tag + " a.txt"})
    result = reply.get("result", reply) if isinstance(reply, dict) else {}
    check("files_read_digest", isinstance(result, dict) and (result.get("digest") == digest("second " + tag) or json.dumps(result).find(digest("second " + tag)) >= 0), brief(reply))
    token_text = "abtest" + secrets.token_hex(8)
    reply = command("files.create", named(tag + " " + token_text + ".txt", token_text))
    check("files_search_fixture", observed(reply), brief(reply))
    found = wait_for(lambda: token_text in json.dumps(query("files.search", {"query": tag + " " + token_text + ".txt"})), 40, 2)
    check("files_search_finds_by_name", found)
    outside_result = query("files.search", {"query": "secret.txt"})
    check("files_search_stays_in_roots", outside not in json.dumps(outside_result))
    reply = command("files.rename", {"root": root, "path": folder + "/" + tag + " a.txt", "parent": folder, "name": tag + " b.txt"})
    check("files_rename_observed", observed(reply) and os.path.exists(os.path.join(path, tag + " b.txt")), brief(reply))
    os.makedirs(os.path.join(path, "target"))
    reply = command("files.move", {"root": root, "path": folder + "/" + tag + " b.txt", "parent": folder + "/target", "name": tag + " c.txt"})
    moved = os.path.join(path, "target", tag + " c.txt")
    check("files_move_observed", observed(reply) and os.path.exists(moved), brief(reply))
    reply = command("files.create", named(tag + " taken.txt", "keep"))
    reply = command("files.rename", {"root": root, "path": folder + "/target/" + tag + " c.txt", "parent": folder, "name": tag + " taken.txt"})
    check("files_rename_never_replaces", open(os.path.join(path, tag + " taken.txt")).read() == "keep" and os.path.exists(moved), brief(reply))
    settled = wait_for(lambda: json.dumps(query("files.read", {"root": root, "path": folder + "/target/" + tag + " c.txt"})).find("digest") >= 0, 30)
    check("files_read_after_move", settled)
    evicted = None
    for _ in range(6):
        evicted = command("files.evict", {"root": root, "path": folder + "/target/" + tag + " c.txt"})
        if observed(evicted):
            break
    if check("files_evict_observed", observed(evicted), brief(evicted)):
        flags = subprocess.run(["ls", "-lO", moved], capture_output=True, text=True).stdout
        check("files_evict_left_a_dataless_item", "dataless" in flags)
        reply = command("files.download", {"root": root, "path": folder + "/target/" + tag + " c.txt"})
        check("files_download_observed", observed(reply), brief(reply))
        check("files_download_restored_the_bytes", open(moved).read() == "second " + tag)
    reply = command("files.trash", {"root": root, "path": folder + "/target/" + tag + " c.txt"})
    check("files_trash_observed", observed(reply) and not os.path.exists(moved), brief(reply))
    if isinstance(reply, dict):
        trash = (reply.get("evidence") or {}).get("trash_path")
        if trash:
            ledger("trash", trash)
    check("files_outside_still_intact", not wrote_outside())
    subprocess.run(["rm", "-rf", outside])


def running(name):
    return subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL).returncode == 0


def run_mac():
    for name, op_name, args, codes in [
        ("volume_high", "mac.set_volume", {"volume": 101}, ("invalid",)),
        ("volume_negative", "mac.set_volume", {"volume": -1}, ("invalid",)),
        ("appearance", "mac.set_appearance", {"appearance": "auto"}, ("invalid",)),
        ("pane", "mac.open_settings", {"pane": "other"}, ("invalid",)),
        ("notify_empty", "mac.notify", {"title": "", "body": "b"}, ("invalid",)),
        ("notify_url", "mac.notify", {"title": tag, "body": "b", "action_url": "file:///etc/hosts"}, ("invalid",)),
        ("keep_awake_long", "mac.keep_awake", {"on": True, "duration": 86401}, ("invalid",)),
        ("lock_screen", "mac.lock_screen", {}, ("unsupported", "forbidden")),
        ("restart", "mac.restart", {}, ("unsupported", "forbidden")),
        ("shutdown", "mac.shutdown", {}, ("unsupported", "forbidden")),
        ("sleep_display", "mac.sleep_display", {}, ("unsupported", "forbidden")),
    ]:
        reply = command(op_name, args)
        check("mac_refused_%s" % name, refused(reply, *codes), brief(reply))
    devices = [r for r in records("mac.audio_device", 100) if not r.get("deleted")]
    output = next((r["data"] for r in devices if r["data"].get("default") and r["data"].get("output") and "volume" in r["data"]), None)
    if output is not None:
        before = round(output["volume"] * 100)
        goal = before - 1 if before > 1 else before + 1
        reply = command("mac.set_volume", {"volume": goal})
        check("mac_set_volume_observed", observed(reply), brief(reply))
        reply = command("mac.set_volume", {"volume": before})
        check("mac_set_volume_restored", observed(reply), brief(reply))
        muted = bool(output.get("muted"))
        reply = command("mac.set_mute", {"muted": muted})
        check("mac_set_mute_same_value_observed", observed(reply), brief(reply))
    else:
        print("INFO no default output with a volume; volume not run")
    display = next((r["data"] for r in records("mac.display", 5)), {})
    if display.get("appearance") in ("light", "dark"):
        reply = command("mac.set_appearance", {"appearance": display["appearance"]})
        check("mac_set_appearance_same_value_observed", observed(reply), brief(reply))
    bundle = "com.apple.calculator"
    had = running("Calculator")
    reply = command("mac.launch_app", {"bundle_id": bundle})
    check("mac_launch_app_observed", observed(reply) and running("Calculator"), brief(reply))
    reply = command("mac.activate_app", {"bundle_id": bundle})
    check("mac_activate_app_observed", observed(reply), brief(reply))
    if not had:
        reply = command("mac.quit_app", {"bundle_id": bundle})
        check("mac_quit_app_observed", observed(reply) and not running("Calculator"), brief(reply))
    reply = command("mac.keep_awake", {"on": True, "duration": 5})
    check("mac_keep_awake_on_observed", observed(reply), brief(reply))
    reply = command("mac.keep_awake", {"on": False})
    check("mac_keep_awake_off_observed", observed(reply), brief(reply))
    before = query("mac.clipboard_read", {})
    saved = before.get("result_json", before.get("result", before)) if isinstance(before, dict) else {}
    if isinstance(saved, dict) and isinstance(saved.get("text"), str):
        reply = command("mac.clipboard_write", {"text": tag})
        check("mac_clipboard_write_observed", observed(reply), brief(reply))
        reply = command("mac.clipboard_write", {"text": saved["text"]})
        check("mac_clipboard_restored", observed(reply), brief(reply))
    elif isinstance(before, dict) and before.get("error"):
        reply = command("mac.clipboard_write", {"text": tag})
        check("mac_clipboard_write_refused_without_always_allow", isinstance(reply, dict) and reply.get("error"), brief(reply))
    else:
        print("INFO mac.clipboard_write not run: the clipboard could not be read back to restore it")
    reply = command("mac.notify", {"title": tag, "body": "Apple Bridge test notification"})
    print("INFO mac.notify", brief(reply))
    had_settings = running("System Settings")
    reply = command("mac.open_settings", {"pane": "general"})
    check("mac_open_settings_observed", observed(reply), brief(reply))
    if not had_settings:
        subprocess.run(["pkill", "-TERM", "-x", "System Settings"])
    for name in ("mac.time_machine_status", "mac.bluetooth_devices", "mac.network_check"):
        reply = query(name, {})
        check("mac_query_%s" % name.split(".")[1], isinstance(reply, dict) and not reply.get("error"), brief(reply))
    reply = query("mac.recognize_song", {"audio_blob": encoded("x")})
    check("mac_query_recognize_song_limited", "limited" in json.dumps(reply), brief(reply))
    reply = query("mac.time_machine_status", {"extra": 1})
    check("mac_query_extra_args_refused", isinstance(reply, dict) and reply.get("error"), brief(reply))
    reply = command("mac.iwork_export", {"app": "pages", "format": "pdf", "document_blob": encoded("[ab-test] x")})
    print("INFO mac.iwork_export", brief(reply))
    print("INFO mac.set_wallpaper not run: the current wallpaper cannot be restored")


def run_shortcuts():
    for name, args in [
        ("flag_short", {"shortcut": "-h"}),
        ("flag_long", {"shortcut": "--output-path=/tmp/ab-test-x"}),
        ("blank", {"shortcut": "   "}),
        ("long_timeout", {"shortcut": "AB Echo", "timeout_ms": 300001}),
        ("two_inputs", {"shortcut": "AB Echo", "input": "x", "input_base64": encoded("x")}),
        ("big_input", {"shortcut": "AB Echo", "input": "x" * 16385}),
    ]:
        reply = command("shortcuts.run", args)
        check("shortcuts_refused_%s" % name, refused(reply, "invalid"), brief(reply))
    check("shortcuts_flag_wrote_nothing", not os.path.exists("/tmp/ab-test-x"))
    words = tag + " echo"
    reply = command("shortcuts.run", {"shortcut": "AB Echo", "input": words, "timeout_ms": 30000})
    evidence = reply.get("evidence", {}) if isinstance(reply, dict) else {}
    check("shortcuts_run_echo_observed", observed(reply), brief(reply))
    output = base64.b64decode(evidence.get("output_base64", "") or "") if isinstance(evidence, dict) else b""
    print("INFO echo output bytes", len(output), "contains input", words.encode() in output)
    reply = command("shortcuts.run", {"shortcut": tag + " no such shortcut", "timeout_ms": 15000})
    check("shortcuts_missing_is_not_observed", isinstance(reply, dict) and reply.get("disposition") != "locally_observed", brief(reply))


def focus_status():
    rows = records("focus.status", 5)
    return rows[0]["data"] if rows else {}


def run_focus():
    for name, args in [
        ("other_mode", {"mode": "Work", "on": True}),
        ("no_mode", {"on": True}),
        ("no_on", {"mode": "Do Not Disturb"}),
        ("until_past", {"mode": "Do Not Disturb", "on": True, "until": "2001-01-01T00:00:00Z"}),
        ("until_with_off", {"mode": "Do Not Disturb", "on": False, "until": "2099-01-01T00:00:00Z"}),
        ("until_garbage", {"mode": "Do Not Disturb", "on": True, "until": "tomorrow"}),
    ]:
        reply = command("focus.set", args)
        check("focus_refused_%s" % name, refused(reply, "invalid"), brief(reply))
    status = focus_status()
    print("INFO focus authorization", status.get("authorization"), "focused", status.get("focused"))
    if status.get("focused") is not False or os.environ.get("REAL_FOCUS_SET") != "1":
        print("INFO focus.set positive not run: a Focus is on or REAL_FOCUS_SET is not 1")
        return
    reply = command("focus.set", {"mode": "Do Not Disturb", "on": True})
    check("focus_set_on_observed", observed(reply), brief(reply))
    reply = command("focus.set", {"mode": "Do Not Disturb", "on": False})
    check("focus_set_off_observed", observed(reply), brief(reply))
    check("focus_restored_off", wait_for(lambda: focus_status().get("focused") is False, 30))


PDF = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n"


def run_pdf():
    note = {"page": 1, "type": "note", "text": tag, "x": 10, "y": 10, "width": 20, "height": 20}
    data = base64.b64encode(PDF).decode()
    for name, args, codes in [
        ("no_annotations", {"data_base64": data, "annotations": []}, ("invalid",)),
        ("no_document", {"annotations": [note]}, ("invalid",)),
        ("not_a_pdf", {"data_base64": encoded("nope"), "annotations": [note]}, ("invalid",)),
        ("file_without_flag", {"file": "/etc/hosts", "annotations": [note]}, ("invalid",)),
    ]:
        reply = command("pdf.annotate", args)
        check("pdf_refused_%s" % name, refused(reply, *codes), brief(reply))
    reply = command("pdf.annotate", {"data_base64": data, "annotations": [dict(note, x=500)]})
    check("pdf_outside_page_not_observed", isinstance(reply, dict) and reply.get("disposition") != "locally_observed", brief(reply))
    reply = command("pdf.annotate", {"data_base64": data, "annotations": [note]})
    check("pdf_annotate_observed", observed(reply), brief(reply))
    artifact = wait_for(lambda: next((r for r in records("pdf.artifact", 200) if not r.get("deleted") and r["data"].get("annotation_count") == 1), None), 30)
    check("pdf_artifact_reaches_gateway", artifact)
    reply = query("pdf.text", {"data_base64": data})
    check("pdf_text_query", isinstance(reply, dict) and not reply.get("error"), brief(reply))


def run_playback():
    for name, op_name, args in [
        ("sound_path", "capture.play_sound", {"name": "../../../tmp/x", "volume": 0.05}),
        ("sound_unknown", "capture.play_sound", {"name": "Nope", "volume": 0.05}),
        ("speech_empty", "speech.say", {"text": "   "}),
        ("speech_volume", "speech.say", {"text": "x", "volume": 2}),
    ]:
        reply = command(op_name, args)
        check("playback_refused_%s" % name, refused(reply, "invalid"), brief(reply))
    reply = command("capture.play_sound", {"name": "Tink", "volume": 0.05})
    check("capture_play_sound_observed", observed(reply), brief(reply))
    reply = command("speech.say", {"text": "test", "volume": 0.05})
    check("speech_say_observed", observed(reply), brief(reply))


def run_maps():
    for name, args in [
        ("lat", {"lat": 91, "lon": 0}),
        ("mode", {"from": {"lat": 0, "lon": 0}, "to": {"lat": 1, "lon": 1}, "mode": "teleport"}),
        ("half_route", {"from": {"lat": 0, "lon": 0}}),
    ]:
        reply = command("maps.open", args)
        check("maps_refused_%s" % name, refused(reply, "invalid"), brief(reply))
    had = running("Maps")
    reply = command("maps.open", {"lat": 37.33, "lon": -122.03})
    print("INFO maps.open", brief(reply))
    check("maps_open_not_refused", isinstance(reply, dict) and not reply.get("error"), brief(reply))
    if not had:
        subprocess.run(["pkill", "-TERM", "-x", "Maps"])


def run_capture():
    for name, op_name, args in [
        ("seconds_high", "capture.record_audio", {"seconds": 61}),
        ("record_seconds_high", "mac.screen_record", {"seconds": 31}),
        ("both_targets", "capture.screenshot", {"display": 1, "window": 2}),
    ]:
        reply = command(op_name, args)
        check("capture_refused_%s" % name, refused(reply, "invalid"), brief(reply))
    reply = command("capture.record_audio", {"seconds": 1})
    print("INFO capture.record_audio", brief(reply))
    reply = command("capture.camera_photo", {})
    print("INFO capture.camera_photo", brief(reply))
    reply = command("capture.screenshot", {})
    print("INFO capture.screenshot", brief(reply))
    if isinstance(reply, dict) and observed(reply):
        check("capture_screenshot_observed", True)
    handle = tempfile.NamedTemporaryFile(prefix="ab-test-", suffix=".pdf", delete=False)
    handle.write(PDF)
    handle.close()
    reply = command("print.document", {"file": handle.name, "printer": "PDF"})
    print("INFO print.document to PDF", brief(reply))
    os.remove(handle.name)


def calendar_events(title):
    reply = op("search", {"text": title, "collection": "calendar.event", "limit": 200})
    rows = reply.get("records") if isinstance(reply, dict) else None
    return [r for r in rows or [] if not r.get("deleted") and r.get("data", {}).get("title") == title]


def run_calendar():
    calendar = next((r for r in records("calendar.calendar", 100) if r["data"].get("title") == "Bridge Test" and r["data"].get("writable") and not r.get("deleted")), None)
    if not check("calendar_bridge_test_found", calendar):
        return
    start = time.strftime("%Y-%m-%dT%H:00:00Z", time.gmtime(time.time() + 3 * 86400))
    end = time.strftime("%Y-%m-%dT%H:30:00Z", time.gmtime(time.time() + 3 * 86400))
    single = tag + " single"
    reply = command("calendar.create_event", {"calendar_id": calendar["id"], "title": single, "start": start, "end": end})
    check("calendar_create_observed", observed(reply), brief(reply))
    event = wait_for(lambda: next(iter(calendar_events(single)), None), 60)
    if not check("calendar_event_reaches_gateway", event):
        return
    ledger("calendar", event["id"])
    reply = command("calendar.update_event", {"id": event["id"], "span": "this", "title": single + " v2"}, event["version"])
    check("calendar_update_observed", observed(reply), brief(reply))
    updated = wait_for(lambda: next(iter(calendar_events(single + " v2")), None), 60)
    check("calendar_update_reaches_gateway", updated)
    stale = command("calendar.update_event", {"id": event["id"], "span": "this", "title": "stale"}, event["version"])
    check("calendar_stale_version_refused", refused(stale, "conflict"), brief(stale))
    updated = updated or event
    reply = command("calendar.delete_event", {"id": updated["id"], "span": "this"}, updated["version"])
    check("calendar_delete_observed", observed(reply), brief(reply))
    check("calendar_delete_reaches_gateway", wait_for(lambda: not calendar_events(single + " v2"), 60))

    series = tag + " series"
    reply = command("calendar.create_event", {"calendar_id": calendar["id"], "title": series, "start": start, "end": end, "recurrence": [{"frequency": "daily", "interval": 1, "count": 4}]})
    check("calendar_series_create_observed", observed(reply), brief(reply))
    rows = wait_for(lambda: (lambda found: found if len(found) == 4 else None)(calendar_events(series)), 90)
    if not check("calendar_four_occurrences_reach_gateway", rows, len(calendar_events(series))):
        return
    for row in rows:
        ledger("calendar", row["id"])
    second = sorted(rows, key=lambda r: r["data"]["start"])[1]
    moved = series + " one"
    reply = command("calendar.update_event", {"id": second["id"], "span": "this", "title": moved}, second["version"])
    check("calendar_occurrence_update_observed", observed(reply), brief(reply))
    changed = wait_for(lambda: next(iter(calendar_events(moved)), None), 60)
    check("calendar_occurrence_update_reaches_gateway", changed)
    check("calendar_other_occurrences_untouched", wait_for(lambda: len(calendar_events(series)) == 3, 60), len(calendar_events(series)))
    if changed:
        ledger("calendar", changed["id"])
        reply = command("calendar.delete_event", {"id": changed["id"], "span": "this"}, changed["version"])
        check("calendar_occurrence_delete_observed", observed(reply), brief(reply))
        check("calendar_occurrence_delete_reaches_gateway", wait_for(lambda: not calendar_events(moved), 60))
    remaining = sorted(calendar_events(series), key=lambda r: r["data"]["start"])
    check("calendar_three_left_after_occurrence_delete", wait_for(lambda: len(calendar_events(series)) == 3, 60), len(remaining))
    first = sorted(calendar_events(series), key=lambda r: r["data"]["start"])[0]
    reply = command("calendar.delete_event", {"id": first["id"], "span": "future"}, first["version"])
    check("calendar_series_delete_observed", observed(reply), brief(reply))
    check("calendar_series_gone", wait_for(lambda: not calendar_events(series), 90), len(calendar_events(series)))


RUNNERS = {"calendar": run_calendar, "mail": run_mail, "files": run_files, "mac": run_mac, "shortcuts": run_shortcuts, "focus": run_focus, "pdf": run_pdf, "playback": run_playback, "maps": run_maps, "capture": run_capture}


def main():
    for source in options.sources:
        RUNNERS[source]()
    found = prompted()
    check("no_permission_prompt", not found, found[:2])
    passed = sum(1 for _, ok in results if ok)
    print("summary %d/%d tag %s" % (passed, len(results), tag))
    sys.exit(0 if passed == len(results) else 1)


main()
