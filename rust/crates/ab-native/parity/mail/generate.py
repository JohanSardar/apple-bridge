import json
import pathlib
import sqlite3
import sys
root=pathlib.Path(sys.argv[1]);operation=sys.argv[2] if len(sys.argv)>2 else 'initial';root.mkdir(parents=True,exist_ok=True)
url='imap://user%40test@EXAMPLE.invalid/My%20Folder/'
def initialize(path,small=False):
    with sqlite3.connect(path) as db:
        db.executescript("CREATE TABLE mailboxes(url TEXT);CREATE TABLE subjects(subject TEXT);CREATE TABLE addresses(address TEXT);CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT);INSERT INTO subjects VALUES('synthetic subject <&>');INSERT INTO addresses VALUES('sender@synthetic.invalid');INSERT INTO mailboxes VALUES('imap://one@synthetic.invalid/INBOX'),('imap://two@synthetic.invalid/Sent'),('imap://three@synthetic.invalid/Archive');")
        db.execute('INSERT INTO mailboxes VALUES(?)',(url,))
        if small:db.execute("INSERT INTO messages(ROWID,mailbox,subject,sender,date_received,read,message_id) VALUES(2000,1,1,1,1700000000,0,'<synthetic-3@synthetic.invalid>')")
        else:db.executemany('INSERT INTO messages VALUES(?,?,?,?,?,?)',[(i%3+1,1,1,1700000000.125 if i%2 else 1700000000,0,f'<synthetic-{i}@synthetic.invalid>') for i in range(1,1001)])
if operation=='initial':
    initialize(root/'store.db')
    (root/'context.json').write_text(json.dumps(dict(observed_at='2026-05-29T00:00:00.000Z',checkpoint='synthetic-checkpoint',scan_id='synthetic-scan')))
elif operation=='replace':
    replacement=root/'replacement.db';initialize(replacement,True);replacement.replace(root/'store.db')
else:
    with sqlite3.connect(root/'store.db') as db:
        if operation=='remove':db.execute('DELETE FROM messages WHERE ROWID=1000')
        elif operation=='update':db.executescript("UPDATE messages SET read=1,mailbox=2 WHERE ROWID=1;INSERT INTO messages VALUES(1,1,1,1700000001,0,'<new@synthetic.invalid>');")
        elif operation=='invalid-url':db.execute("UPDATE mailboxes SET url='broken' WHERE ROWID=4")
        elif operation=='repair-url':db.execute('UPDATE mailboxes SET url=? WHERE ROWID=4',(url,))
        elif operation=='alias':db.execute("UPDATE messages SET message_id='invalid alias' WHERE ROWID=2")
        elif operation=='missing-address':db.execute('UPDATE messages SET sender=999 WHERE ROWID=3')
        elif operation=='missing-mailbox':db.execute('UPDATE messages SET mailbox=999 WHERE ROWID=4')
        elif operation=='repair-mailbox':db.execute('UPDATE messages SET mailbox=2 WHERE ROWID=4')
        elif operation=='clear':db.execute('DELETE FROM messages')
        elif operation=='invalid-text':db.execute("UPDATE subjects SET subject=CAST(x'80' AS TEXT)")
        elif operation=='repair-text':db.execute('UPDATE subjects SET subject=?',('synthetic\0 subject <&>',))
        else:raise ValueError(operation)
