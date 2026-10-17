import os
import re
import struct
import sys
from collections import defaultdict
from datetime import datetime, timezone, timedelta

MAC_EPOCH = 978307200
MAX_SESSION = 4 * 3600
RECORD = re.compile(rb"\x10(.)\x18([\x00\x01])\x21(.{8})\x32(.)", re.S)


def events(path):
    data = open(path, "rb").read()
    for m in RECORD.finditer(data):
        n = m.group(4)[0]
        start = m.end()
        bundle = data[start:start + n]
        if not re.fullmatch(rb"[A-Za-z0-9._-]+", bundle or b"-"):
            continue
        ts = struct.unpack("<d", m.group(3))[0] + MAC_EPOCH
        if 1.6e9 < ts < 2.1e9:
            yield ts, m.group(2) == b"\x01", bundle.decode()


def usage(files, tz):
    evs = sorted({e for f in files for e in events(f)})
    totals = defaultdict(lambda: defaultdict(float))
    cur, since = None, None

    def close(t):
        if cur and since and t > since:
            totals[datetime.fromtimestamp(since, tz).date()][cur] += min(t - since, MAX_SESSION)

    for ts, starting, app in evs:
        if starting:
            close(ts)
            cur, since = app, ts
        elif app == cur:
            close(ts)
            cur, since = None, None
    return totals, evs


def main(root, tz_hours=-5):
    tz = timezone(timedelta(hours=tz_hours))
    groups = {}
    for dirpath, _, names in os.walk(root):
        if "tombstone" in dirpath:
            continue
        files = [os.path.join(dirpath, n) for n in names if n.isdigit()]
        if files:
            groups[os.path.relpath(dirpath, root)] = files
    for device, files in sorted(groups.items()):
        totals, evs = usage(files, tz)
        if not evs:
            continue
        first = datetime.fromtimestamp(evs[0][0], tz)
        last = datetime.fromtimestamp(evs[-1][0], tz)
        print(f"\n### {device}: {len(evs)} events, {first:%b %d %H:%M} to {last:%b %d %H:%M}")
        for day in sorted(totals)[-3:]:
            apps = sorted(totals[day].items(), key=lambda kv: -kv[1])
            total = sum(v for _, v in apps)
            print(f"  {day}  total {total / 3600:.1f} h")
            for app, secs in apps[:10]:
                print(f"    {secs / 60:6.0f} min  {app}")


if __name__ == "__main__":
    main(sys.argv[1])
