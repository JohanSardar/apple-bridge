import json
import pathlib
import sqlite3
import struct
import sys
root=pathlib.Path(sys.argv[1]);operation=sys.argv[2] if len(sys.argv)>2 else 'initial';root.mkdir(parents=True,exist_ok=True)

def atom(kind,payload):return struct.pack('>I',len(payload)+8)+kind.encode()+payload

def audio(transcript=False):
    data=atom('mdat',bytes(256))
    if transcript:
        payload=b'{"attributedString":{"runs":["Synthetic",0],"attributeTable":[{"timeRange":[0,1.5]}]}}'
        data+=atom('moov',atom('trak',atom('udta',atom('tsrp',payload))))
    return data

if operation=='initial':
    with sqlite3.connect(root/'store.db') as db:
        db.executescript("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic-vmemo-1');CREATE TABLE ZCLOUDRECORDING(Z_PK INTEGER,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER);INSERT INTO ZCLOUDRECORDING VALUES(1,'fixture','Synthetic',800000000,0,NULL,1);INSERT INTO ZCLOUDRECORDING VALUES(2,'','Deleted <&>',800000000.125,5.25,800000010,1);")
        db.execute('UPDATE ZCLOUDRECORDING SET ZCUSTOMLABEL=? WHERE Z_PK=1',('Synthetic\0Fixture',))
    (root/'context.json').write_text(json.dumps(dict(observed_at='2026-05-29T00:00:00.000Z',checkpoint='synthetic-checkpoint',scan_id='synthetic-scan')))
elif operation=='asset-schema':
    with sqlite3.connect(root/'store.db') as db:db.executescript("ALTER TABLE ZCLOUDRECORDING ADD COLUMN ZPATH TEXT; UPDATE ZCLOUDRECORDING SET ZPATH='fixture.m4a' WHERE Z_PK=1; CREATE TABLE ZFOLDER(Z_PK INTEGER,ZENCRYPTEDNAME TEXT);INSERT INTO ZFOLDER VALUES(1,'Synthetic');INSERT INTO ZFOLDER VALUES(2,'Empty');")
elif operation=='audio':(root/'fixture.m4a').write_bytes(audio())
elif operation=='transcript':(root/'fixture.m4a').write_bytes(audio(True))
elif operation=='mutate-audio':(root/'fixture.m4a').write_bytes(atom('mdat',b'new synthetic audio'))
elif operation=='remove-audio':(root/'fixture.m4a').unlink()
elif operation=='escape':
    with sqlite3.connect(root/'store.db') as db:db.execute("UPDATE ZCLOUDRECORDING SET ZPATH='../escape.m4a' WHERE Z_PK=1")
    (root.parent/'escape.m4a').write_bytes(audio(True))
elif operation=='symlink':
    (root/'link.m4a').symlink_to(root.parent/'escape.m4a')
    with sqlite3.connect(root/'store.db') as db:db.execute("UPDATE ZCLOUDRECORDING SET ZPATH='link.m4a' WHERE Z_PK=1")
elif operation=='restore':
    (root/'fixture.m4a').write_bytes(audio(True))
    with sqlite3.connect(root/'store.db') as db:db.execute("UPDATE ZCLOUDRECORDING SET ZPATH='fixture.m4a' WHERE Z_PK=1")
elif operation=='delete':
    with sqlite3.connect(root/'store.db') as db:db.executescript('DELETE FROM ZCLOUDRECORDING WHERE Z_PK=1;DELETE FROM ZFOLDER WHERE Z_PK=2;')
elif operation=='clear':
    with sqlite3.connect(root/'store.db') as db:db.executescript('DELETE FROM ZCLOUDRECORDING;DELETE FROM ZFOLDER;')
elif operation=='negative':
    with sqlite3.connect(root/'store.db') as db:db.execute('UPDATE ZCLOUDRECORDING SET ZDURATION=-1 WHERE Z_PK=1')
elif operation=='repair-negative':
    with sqlite3.connect(root/'store.db') as db:db.execute('UPDATE ZCLOUDRECORDING SET ZDURATION=42 WHERE Z_PK=1')
elif operation=='invalid-text':
    with sqlite3.connect(root/'store.db') as db:db.execute("UPDATE ZCLOUDRECORDING SET ZCUSTOMLABEL=CAST(x'80' AS TEXT) WHERE Z_PK=1")
elif operation=='repair-text':
    with sqlite3.connect(root/'store.db') as db:db.execute('UPDATE ZCLOUDRECORDING SET ZCUSTOMLABEL=? WHERE Z_PK=1',('Synthetic\0Fixture',))
elif operation=='uuid':
    with sqlite3.connect(root/'store.db') as db:db.execute("UPDATE Z_METADATA SET Z_UUID='synthetic-vmemo-2'")
else:raise ValueError(operation)
