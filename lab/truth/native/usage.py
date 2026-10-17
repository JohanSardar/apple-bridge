import datetime
import json
import math
import os
import re
import sqlite3
import struct
import subprocess
import sys
from collections import defaultdict
from zoneinfo import ZoneInfo

root, timezone_name, output_path = sys.argv[1:4]
zone = ZoneInfo(timezone_name)
connection = sqlite3.connect("file:" + os.path.join(root, "sync.db") + "?mode=ro", uri=True)
peers = list(connection.execute("select device_identifier,name,platform,model,me from DevicePeer"))
connection.close()
pattern = re.compile(rb"\x10.\x18([\x00\x01])\x21(.{8})\x32(.)", re.S)
records = []
for identifier, name, platform, model, local in peers:
    hardware = (model or "").lower()
    kind = "mac" if platform == 3 else "ipad" if "ipad" in hardware or hardware.startswith("j") else "iphone"
    folder = os.path.join(root, "streams", "local" if local else "remote/" + identifier)
    if not os.path.isdir(folder) and not local:
        remote = os.path.join(root, "streams", "remote")
        if os.path.isdir(remote):
            folder = next((os.path.join(remote, path) for path in os.listdir(remote) if path.lower() == identifier.lower()), folder)
    events = set()
    for base, dirs, names in os.walk(folder):
        dirs[:] = [directory for directory in dirs if "tombstone" not in directory]
        for filename in names:
            if not filename.isdigit():
                continue
            data = open(os.path.join(base, filename), "rb").read()
            for match in pattern.finditer(data):
                length = match.group(3)[0]
                bundle = data[match.end():match.end() + length]
                if len(bundle) != length or not re.fullmatch(rb"[A-Za-z0-9._-]+", bundle):
                    continue
                at = struct.unpack("<d", match.group(2))[0] + 978307200
                if math.isfinite(at) and 1.6e9 < at < 2.1e9:
                    events.add((at, match.group(1) == b"\x01", bundle.decode()))
    ordered = sorted(events)
    def timestamp(at):
        return datetime.datetime.fromtimestamp(at, datetime.timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    latest = timestamp(ordered[-1][0]) if ordered else ""
    build = subprocess.check_output(["sysctl", "-n", "kern.osversion"], text=True).strip() if local else ""
    device = {"name": name or "", "platform": kind, "build": build, "is_local": bool(local), "latest_event_at": latest}
    records.append({"collection": "usage.device", "native_key": identifier, "data": device})
    totals = defaultdict(lambda: [0.0, 0, 0.0, 0.0])
    current = None
    previous = None
    def allocate(since, until, application, locked):
        end = min(until, since + 14400)
        cursor = since
        while cursor < end:
            date = datetime.datetime.fromtimestamp(cursor, zone)
            midnight = datetime.datetime.combine(date.date() + datetime.timedelta(days=1), datetime.time(), zone).timestamp()
            through = min(end, midnight)
            if through <= cursor:
                raise ValueError("invalid day boundary")
            value = totals[(date.date().isoformat(), application)]
            value[2 if locked else 0] += through - cursor
            if not locked:
                value[1] += 1
            value[3] = max(value[3], until)
            cursor = through
    for at, starting, application in ordered:
        closes = starting or (current is not None and current[1] == application)
        if closes and current is not None:
            since, active = current
            if at > since:
                if kind == "mac" and active in ("loginwindow", "com.apple.loginwindow", "com.apple.ScreenSaver.Engine"):
                    if previous is not None:
                        allocate(since, at, previous, True)
                else:
                    allocate(since, at, active, False)
                    previous = active
        if starting:
            current = (at, application)
        elif closes:
            current = None
    for (day, application), (seconds, sessions, clipped, newest) in sorted(totals.items()):
        value = {"device_id": {"$ref": {"collection": "usage.device", "native_key": identifier}}, "day": day, "time_zone": timezone_name, "bundle_id": application, "seconds_estimated": round(seconds, 3), "sessions": sessions, "clipped_seconds": round(clipped, 3), "provenance": "biome.app_infocus", "decoder_version": "infocus-1", "latest_event_at": timestamp(newest), "uncertainty": "Focus transitions only; sessions capped at 4h; open sessions omitted; Mac lock focus excluded and attributed as clipped time; historical sleep gaps and settings parity unproven; remote batches may lag"}
        records.append({"collection": "usage.app_day", "native_key": identifier + "/" + day + "/" + application, "data": value})
with os.fdopen(os.open(output_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as output:
    json.dump({"status": "verified", "records": records}, output, separators=(",", ":"))
