import datetime
import json
import pathlib
import plistlib
import sqlite3
import sys

root = pathlib.Path(sys.argv[1])
operation = sys.argv[2] if len(sys.argv) > 2 else 'initial'
root.mkdir(parents=True, exist_ok=True)
now = 1_780_000_000.0

def history(path, empty=False):
    with sqlite3.connect(path) as db:
        db.executescript('CREATE TABLE history_items(id INTEGER PRIMARY KEY,url TEXT,visit_count INTEGER); CREATE TABLE history_visits(id INTEGER PRIMARY KEY,history_item INTEGER,visit_time REAL,title TEXT);')
        if not empty:
            db.executemany('INSERT INTO history_items VALUES(?,?,?)', [(1,'https://synthetic.invalid/a?<x>&q=\u2028',99),(2,'https://synthetic.invalid/old',7)])
            db.executemany('INSERT INTO history_visits VALUES(?,?,?,?)', [(1,1,now-978307200-2,'earlier'),(2,1,now-978307200-1,'tie first'),(3,1,now-978307200-1,'latest <&>'),(4,2,now-978307200-8640000,'old')])

def bookmark(format=plistlib.FMT_BINARY, empty=False):
    def leaf(key, **extra):
        return dict(WebBookmarkType='WebBookmarkTypeLeaf',WebBookmarkUUID=key,URLString='https://synthetic.invalid/'+key,Title='fallback',URIDictionary={'title':'<&> '+key},**extra)
    children = [] if empty else [dict(WebBookmarkType='WebBookmarkTypeList',Title='Favorites',Children=[leaf('bookmark-1')]),dict(WebBookmarkType='WebBookmarkTypeList',Title='com.apple.ReadingList',Children=[leaf('reading-1',ReadingList={'DateAdded':datetime.datetime(2026,1,2,3,4,5)}),leaf('reading-2',ReadingList={'DateLastViewed':datetime.datetime(2026,1,3)})]),dict(WebBookmarkType='WebBookmarkTypeProxy')]
    (root/'Bookmarks.plist').write_bytes(plistlib.dumps({'Children':children},fmt=format))

if operation == 'initial':
    history(root/'History.db')
    with sqlite3.connect(root/'CloudTabs.db') as db:
        db.executescript('CREATE TABLE cloud_tabs(tab_uuid TEXT,device_uuid TEXT,title TEXT,url TEXT); CREATE TABLE cloud_tab_devices(device_uuid TEXT,device_name TEXT);')
        db.execute('INSERT INTO cloud_tab_devices VALUES(?,?)',('device-1','Synthetic Device'))
        db.executemany('INSERT INTO cloud_tabs VALUES(?,?,?,?)',[('tab-1','device-1','Cloud <&>','https://synthetic.invalid/cloud'),('tab-2','unknown','Unknown','')])
    bookmark()
    (root/'context.json').write_text(json.dumps(dict(now=now,generation='safari-fixture-generation',checkpoint='synthetic-checkpoint',scan_id='synthetic-scan',observed_at='2026-05-29T00:00:00.000Z',automation_permitted=True)))
elif operation == 'history-title':
    with sqlite3.connect(root/'History.db') as db: db.execute("UPDATE history_visits SET title='Changed / <&> \u2029' WHERE id=3")
elif operation == 'history-clear':
    with sqlite3.connect(root/'History.db') as db: db.execute('DELETE FROM history_visits')
elif operation == 'history-add':
    with sqlite3.connect(root/'History.db') as db:
        db.execute('INSERT OR IGNORE INTO history_items VALUES(1,?,99)',('https://synthetic.invalid/a?<x>&q=\u2028',))
        db.execute('INSERT INTO history_visits VALUES(5,1,?,?)',(now-978307200,'Back'))
elif operation == 'history-replace':
    path=root/'replacement.db'; history(path,True); path.replace(root/'History.db')
elif operation == 'history-broken':
    with sqlite3.connect(root/'History.db') as db: db.execute('ALTER TABLE history_visits RENAME TO missing_visits')
elif operation == 'history-repair':
    with sqlite3.connect(root/'History.db') as db: db.execute('ALTER TABLE missing_visits RENAME TO history_visits')
elif operation == 'bookmarks-empty': bookmark(empty=True)
elif operation == 'bookmarks-xml': bookmark(format=plistlib.FMT_XML)
elif operation == 'bookmarks-broken': (root/'Bookmarks.plist').write_bytes(b'<broken>')
elif operation == 'cloud-clear':
    with sqlite3.connect(root/'CloudTabs.db') as db: db.execute('DELETE FROM cloud_tabs WHERE tab_uuid=?',('tab-1',))
elif operation == 'expiry':
    path=root/'context.json'; context=json.loads(path.read_text());context['now']+=91*86400;path.write_text(json.dumps(context));(root/'History.db').touch()
else: raise ValueError(operation)
