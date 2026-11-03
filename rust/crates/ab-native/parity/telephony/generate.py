import json
import pathlib
import sqlite3
import sys
root=pathlib.Path(sys.argv[1]);kind=sys.argv[2];operation=sys.argv[3] if len(sys.argv)>3 else 'initial'
root.mkdir(parents=True,exist_ok=True)
path=root/'store.db'
if operation=='initial':
    with sqlite3.connect(path) as db:
        db.executescript("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic-uuid-1');")
        if kind=='calls':
            db.execute('CREATE TABLE ZCALLRECORD(Z_PK INTEGER,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE REAL)')
            db.executemany('INSERT INTO ZCALLRECORD VALUES(?,?,?,?,?,?,?,?,?)',[(1,'+15550000001','Synthetic <&>',800000000.125,1.25,1,1,'com.apple.telephony',0),(2,'synthetic@invalid','FT Video',800000001.5,2,0,1,'FaceTime',8),(3,'+15550000003','Missed',800000002,0,0,0,'',0),(10,'synthetic','FT Audio',None,None,None,None,'com.apple.facetime',1)])
        else:
            db.execute('CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE REAL,ZPROVIDER TEXT,ZMESSAGETYPE REAL)')
            db.executemany('INSERT INTO ZSTOREDMESSAGE VALUES(?,?,?,?,?,?,?,?,?)',[(1,'vm-1','+15550000001',800000000.125,4.25,0,0,'com.apple.coretelephony',0),(2,'vm-2','synthetic@invalid',800000001.5,8,1,1,'test.FaceTimeProvider',2),(3,'','unknown <&>',800000002,0,0,2,'test.FaceTimeProvider',1),(10,None,None,None,None,None,99,'unknown',0)])
    (root/'context.json').write_text(json.dumps(dict(observed_at='2026-05-29T00:00:00.000Z',checkpoint='fixture-checkpoint',scan_id='fixture-scan')))
else:
    table='ZCALLRECORD' if kind=='calls' else 'ZSTOREDMESSAGE'
    with sqlite3.connect(path) as db:
        if operation=='update':db.execute(f'UPDATE {table} SET ZDURATION=12.875 WHERE Z_PK=1')
        elif operation=='remove':db.execute(f'DELETE FROM {table} WHERE Z_PK=2')
        elif operation=='duplicate':db.execute(f'INSERT INTO {table} SELECT * FROM {table} WHERE Z_PK=1')
        elif operation=='repair-duplicate':db.execute(f'DELETE FROM {table} WHERE rowid=(SELECT MAX(rowid) FROM {table} WHERE Z_PK=1)')
        elif operation=='negative':db.execute(f'UPDATE {table} SET ZDURATION=-1 WHERE Z_PK=1')
        elif operation=='repair-negative':db.execute(f'UPDATE {table} SET ZDURATION=12.875 WHERE Z_PK=1')
        elif operation=='uuid':
            db.execute("UPDATE Z_METADATA SET Z_UUID='synthetic-uuid-2'")
            db.execute(f'DELETE FROM {table} WHERE Z_PK=3')
        elif operation=='schema':db.execute(f'ALTER TABLE {table} RENAME COLUMN ZDURATION TO missing_duration')
        elif operation=='repair-schema':db.execute(f'ALTER TABLE {table} RENAME COLUMN missing_duration TO ZDURATION')
        elif operation=='clear':db.execute(f'DELETE FROM {table}')
        elif operation=='replace':
            replacement=root/'replacement.db'
            with sqlite3.connect(replacement) as target:db.backup(target)
        else:raise ValueError(operation)
    if operation=='replace':replacement.replace(path)
