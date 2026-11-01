import email.policy
import email.parser
import email.utils
import hashlib
import json
import pathlib
import sqlite3
import stat
import sys

root = pathlib.Path.home() / 'Library/Mail/V10'
connection = sqlite3.connect((root / 'MailData/Envelope Index').as_uri() + '?mode=ro', uri=True)
connection.execute('PRAGMA query_only=ON')
if len(sys.argv) != 2 or not sys.argv[1].startswith('[ab-test] run_'):
    raise SystemExit('Mail body qualification requires its own tagged fixture')
rows = connection.execute("SELECT m.ROWID, coalesce(s.subject,''), coalesce(a.address,''), m.date_received FROM messages m JOIN subjects s ON s.ROWID=m.subject LEFT JOIN addresses a ON a.ROWID=m.sender WHERE s.subject = ? ORDER BY m.ROWID", (sys.argv[1],))
samples = []
for key, subject, sender, received in rows:
    files = list(root.rglob(str(key) + '.emlx'))
    if len(files) != 1:
        continue
    metadata = files[0].lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_flags & stat.SF_DATALESS or metadata.st_size > 32 << 20:
        continue
    with files[0].open('rb') as stream:
        length = int(stream.readline())
        raw = stream.read(length)
    if len(raw) != length:
        continue
    message = email.parser.BytesParser(policy=email.policy.default).parsebytes(raw)
    if message.is_multipart() or message.get_content_type() != 'text/plain':
        continue
    senders = {address.lower() for _, address in email.utils.getaddresses(message.get_all('From', [])) if address}
    recipients = {address.lower() for _, address in email.utils.getaddresses(message.get_all('To', [])) if address}
    if not senders or not senders.issubset(recipients) or sender.lower() not in senders:
        continue
    text = message.get_content().replace('\r\n', '\n')
    samples.append({'subject_digest': hashlib.sha256(subject.encode()).hexdigest(), 'sender_digest': hashlib.sha256(sender.encode()).hexdigest(), 'received': received, 'body_digest': hashlib.sha256(text.encode()).hexdigest()})
print(json.dumps({'samples': samples}))
