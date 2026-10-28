import datetime
import hashlib
import json
import pathlib
import sqlite3
import sys

cutoff = int(sys.argv[1])
path = pathlib.Path.home() / 'Library/Mail/V10/MailData/Envelope Index'
try:
    connection = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)
    connection.execute('PRAGMA query_only=ON')
    connection.execute('BEGIN')
    counts = {str(key): count for key, count in connection.execute('SELECT b.ROWID, count(m.ROWID) FROM mailboxes b LEFT JOIN messages m ON m.mailbox=b.ROWID AND m.date_received >= ? GROUP BY b.ROWID', (cutoff,))}
    samples = []
    for subject, sender, received in connection.execute("SELECT coalesce(s.subject,''), coalesce(a.address,''), m.date_received FROM messages m LEFT JOIN subjects s ON s.ROWID=m.subject LEFT JOIN addresses a ON a.ROWID=m.sender WHERE m.date_received >= ? ORDER BY random() LIMIT 25", (cutoff,)):
        date = datetime.datetime.fromtimestamp(received, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
        digest = hashlib.sha256(json.dumps([subject, sender, date], ensure_ascii=False, separators=(',', ':')).replace('\u2028', r'\u2028').replace('\u2029', r'\u2029').encode()).hexdigest()
        samples.append({'digest': digest})
    print(json.dumps({'counts': counts, 'samples': samples}))
except (sqlite3.Error, OSError, ValueError, TypeError, UnicodeError):
    sys.exit('Mail Truth unavailable: Full Disk Access for the terminal running bridge-lab and a V10 Envelope Index required')
