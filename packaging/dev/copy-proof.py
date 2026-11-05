#!/usr/bin/env python3
"""Runs a build against a private copy of the live home and compares it to the copy's start, by record id.

usage: copy-proof.py --app APP --bridge BRIDGE --dir DIR [--port N] [--keep-switch|--no-switch] [--seconds N]
                     [--sources a,b] [--settle-source x,...]

The live home and live gateway are only read. Prints counts and field names, never values. A Helper whose
signing requirement differs from the reference is never launched, and an AUTHREQ_PROMPTING for the Helper in
the TCC log stops the run.
"""
import argparse, collections, hashlib, json, os, shutil, signal, sqlite3, subprocess, sys, time, urllib.request

LIVE = os.path.expanduser("~/Library/Application Support/AppleBridgeLive")
REFERENCE = os.path.expanduser("~/ab-live/main-030ebf5e/AppleBridgeHelper.app")
HELPER_ID = "dev.applebridge.helper"


def requirement(app):
    out = subprocess.run(["codesign", "-d", "-r-", app], capture_output=True, text=True)
    return next((line for line in (out.stdout + out.stderr).splitlines() if line.startswith("designated")), "")


def backup(source, destination):
    src = sqlite3.connect("file:%s?mode=ro" % source, uri=True, timeout=60)
    dst = sqlite3.connect(destination)
    src.backup(dst)
    dst.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    dst.close()
    src.close()


def seed(home, keep_switch):
    os.makedirs(home)
    for name in ("state.db", "ledger.db"):
        backup(os.path.join(LIVE, name), os.path.join(home, name))
    for entry in os.listdir(LIVE):
        if entry.startswith(("state.db", "ledger.db")) or entry in ("remote", "run", "lock", "credentials.lock", "runtime.json"):
            continue
        source = os.path.join(LIVE, entry)
        subprocess.run(["cp", "-cR", source, os.path.join(home, entry)], check=True)
    for stale in ("helper/helper.pid", "helper/session/ab-session.log"):
        path = os.path.join(home, stale)
        if os.path.exists(path):
            os.remove(path)
    if not keep_switch:
        for name in ("rust-capture.json", "rust-capture.json.prev"):
            path = os.path.join(home, "helper/session", name)
            if os.path.exists(path):
                os.remove(path)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def load(path, sources):
    connection = sqlite3.connect("file:%s?mode=ro" % path, uri=True, timeout=60)
    marks = ",".join("?" * len(sources))
    rows = {}
    query = "select collection,id,deleted,payload from records where source in (%s) and collection not in ('system.source','system.command')" % marks
    for collection, record, deleted, payload in connection.execute(query, sources):
        data = json.loads(payload).get("data")
        rows[(collection, record)] = (bool(deleted), digest(data), data)
    connection.close()
    return rows


def compare(before, after):
    report = {}
    for collection in sorted({key[0] for key in before} | {key[0] for key in after}):
        a = {key: value for key, value in before.items() if key[0] == collection}
        b = {key: value for key, value in after.items() if key[0] == collection}
        fields = collections.Counter()
        row = {
            "before_alive": sum(not v[0] for v in a.values()), "before_tombstones": sum(v[0] for v in a.values()),
            "after_alive": sum(not v[0] for v in b.values()), "after_tombstones": sum(v[0] for v in b.values()),
            "only_before": len(a.keys() - b.keys()), "only_after": len(b.keys() - a.keys()),
            "only_after_alive": sum(not b[k][0] for k in b.keys() - a.keys()),
            "newly_tombstoned": 0, "revived": 0, "data_changed": 0,
        }
        for key in a.keys() & b.keys():
            if a[key][0] != b[key][0]:
                row["revived" if a[key][0] else "newly_tombstoned"] += 1
            elif a[key][1] != b[key][1]:
                row["data_changed"] += 1
                left, right = a[key][2], b[key][2]
                if isinstance(left, dict) and isinstance(right, dict):
                    fields.update(name for name in set(left) | set(right) if left.get(name) != right.get(name))
                else:
                    fields["<root>"] += 1
        if fields:
            row["changed_fields"] = dict(fields.most_common(12))
        report[collection] = row
    return report


def prompted(since):
    out = subprocess.run(["log", "show", "--style", "compact", "--start", since, "--predicate", 'subsystem == "com.apple.TCC" AND eventMessage CONTAINS "AUTHREQ_PROMPTING"'], capture_output=True, text=True, timeout=120)
    return [line[:160] for line in out.stdout.splitlines() if HELPER_ID in line or "AppleBridge" in line]


def state_counts(home, sources):
    connection = sqlite3.connect("file:%s/state.db?mode=ro" % home, uri=True, timeout=20)
    marks = ",".join("?" * len(sources))
    rows = connection.execute("select collection,deleted,count(*) from records where source in (%s) group by 1,2" % marks, sources).fetchall()
    connection.close()
    return sorted(rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", required=True)
    parser.add_argument("--bridge", required=True)
    parser.add_argument("--dir", required=True)
    parser.add_argument("--port", type=int, default=7901)
    parser.add_argument("--seconds", type=int, default=1500)
    parser.add_argument("--quiet-rounds", type=int, default=8)
    parser.add_argument("--no-switch", action="store_true", help="remove rust-capture.json from the copy")
    parser.add_argument("--fresh", action="store_true", help="start from an empty home")
    parser.add_argument("--sources", default="safari,usage,notes,voice_memos,mail,phone,voicemail,calendar,contacts,reminders,messages,files,mac,focus,shortcuts,pdf")
    parser.add_argument("--hold", action="store_true", help="leave the gateway running and write hold.json")
    options = parser.parse_args()
    want = requirement(REFERENCE)
    if not want or requirement(options.app) != want:
        sys.exit("signing requirement differs from the reference; nothing launched")
    subprocess.run(["codesign", "--verify", "--strict", options.app], check=True)
    sources = options.sources.split(",")
    work = os.path.abspath(options.dir)
    home = work + "/home"
    os.makedirs(work, exist_ok=False)
    since = time.strftime("%Y-%m-%d %H:%M:%S")
    live_pid = subprocess.run(["pgrep", "-f", "start --role gateway --home " + LIVE], capture_output=True, text=True).stdout.split()[:1]
    if options.fresh:
        os.makedirs(home + "/helper", mode=0o700)
        before = {}
    else:
        seed(home, not options.no_switch)
        shutil.copy2(home + "/state.db", work + "/seed.db")
        before = load(work + "/seed.db", sources)
    env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE="file")
    log = open(work + "/gateway.log", "ab")
    gateway = subprocess.Popen([options.bridge, "start", "--role", "gateway", "--home", home, "--api-addr", "127.0.0.1:%d" % options.port, "--peer-addr", "127.0.0.1:%d" % (options.port + 1), "--helper-app", options.app], env=env, stdout=log, stderr=log, start_new_session=True, cwd=home)
    started = time.time()
    result, previous, quiet = "timeout", None, 0
    try:
        while time.time() - started < options.seconds:
            time.sleep(15)
            if gateway.poll() is not None:
                result = "gateway exited %s" % gateway.returncode
                break
            found = prompted(since)
            if found:
                result = "PERMISSION PROMPT, stopped: %s" % found[:2]
                break
            counts = state_counts(home, [name for name in sources if name != "mac"])
            states = os.listdir(home + "/helper/session/capture-state") if os.path.isdir(home + "/helper/session/capture-state") else []
            quiet = quiet + 1 if counts == previous else 0
            previous = counts
            print(time.strftime("%H:%M:%S"), "elapsed", int(time.time() - started), "quiet", quiet, "states", len(states), flush=True)
            if quiet >= options.quiet_rounds:
                result = "settled"
                break
        session_log = home + "/helper/session/ab-session.log"
        text = open(session_log, errors="replace").read() if os.path.exists(session_log) else ""
        problems = {needle: text.count(needle) for needle in ("facts link failed", "ownership conflict", "reader:", "capture ended", "panicked", "start pid")}
        report = {"result": result, "seconds": int(time.time() - started), "log": problems}
        if options.hold and result == "settled":
            json.dump({"pid": gateway.pid, "home": home, "api": "127.0.0.1:%d" % options.port}, open(work + "/hold.json", "w"))
        else:
            os.killpg(gateway.pid, signal.SIGTERM)
            for _ in range(30):
                if gateway.poll() is not None:
                    break
                time.sleep(1)
            if gateway.poll() is None:
                os.killpg(gateway.pid, signal.SIGKILL)
        snapshot = work + "/after.db"
        backup(home + "/state.db", snapshot)
        report["compare"] = compare(before, load(snapshot, sources))
        live_now = subprocess.run(["pgrep", "-f", "start --role gateway --home " + LIVE], capture_output=True, text=True).stdout.split()[:1]
        report["live_gateway_untouched"] = live_now == live_pid
        report["prompts"] = prompted(since)
        json.dump(report, open(work + "/report.json", "w"), indent=1, sort_keys=True)
        print(json.dumps({k: v for k, v in report.items() if k != "compare"}, sort_keys=True))
        for collection, row in report["compare"].items():
            print(collection, json.dumps(row, sort_keys=True))
    finally:
        if gateway.poll() is None and not options.hold:
            os.killpg(gateway.pid, signal.SIGKILL)


if __name__ == "__main__":
    main()
