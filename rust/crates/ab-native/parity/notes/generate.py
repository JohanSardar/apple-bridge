import json
import pathlib
import sqlite3
import sys
root=pathlib.Path(sys.argv[1]);operation=sys.argv[2] if len(sys.argv)>2 else 'initial';fixtures=pathlib.Path(sys.argv[3]);root.mkdir(parents=True,exist_ok=True)
path=root/'NoteStore.sqlite'
media=root/'Accounts/account/Media/media/fixture.bin'
original='[ab-test] exact Notes attachment bytes 🧪'.encode()
if operation=='initial':
    with sqlite3.connect(path) as db:
        db.executescript("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('ab-test-notes-store');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount'),(4,'ICAttachment'),(5,'ICMedia');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER DEFAULT 1);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT,ZFILENAME TEXT,ZMERGEABLEDATA1 BLOB,ZALTTEXT TEXT,ZGENERATION TEXT,ZSERVERRECORDDATA BLOB,Z_OPT INTEGER DEFAULT 1,ZSMARTFOLDERQUERYJSON TEXT,ZCREATIONDATE1 REAL,ZMODIFICATIONDATE1 REAL,ZFALLBACKIMAGEGENERATION TEXT);")
        db.executescript("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Bridge Test Account');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8,ZPARENT,ZFOLDERTYPE) VALUES(10,1,'root','Bridge Test',1,NULL,0),(11,1,'trash','Recently Deleted',1,NULL,1),(12,1,'nested','Nested',1,10,0);CREATE TRIGGER object_revision AFTER UPDATE ON ZICCLOUDSYNCINGOBJECT WHEN NEW.Z_OPT=OLD.Z_OPT BEGIN UPDATE ZICCLOUDSYNCINGOBJECT SET Z_OPT=OLD.Z_OPT+1 WHERE Z_PK=NEW.Z_PK; END;CREATE TRIGGER body_revision AFTER UPDATE ON ZICNOTEDATA WHEN NEW.Z_OPT=OLD.Z_OPT BEGIN UPDATE ZICNOTEDATA SET Z_OPT=OLD.Z_OPT+1 WHERE Z_PK=NEW.Z_PK; END;")
        for pk,file in [(1,'full.gz'),(2,'large-body.gz')]:db.execute('INSERT INTO ZICNOTEDATA(Z_PK,ZDATA) VALUES(?,?)',(pk,(fixtures/file).read_bytes()))
        db.executescript("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA,ZISPINNED,ZISPASSWORDPROTECTED) VALUES(20,2,'note','[ab-test] Full',12,1,1,0),(21,2,'locked','[ab-test] Locked',12,1,0,1),(22,2,'deleted','[ab-test] Deleted',11,1,0,0);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(24,2,'large','[ab-test] Large',12,2);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNOTE1,ZMEDIA,ZTYPEUTI1) VALUES(30,4,'fixture-image',20,31,'public.data'),(32,4,'fixture-table',20,NULL,'com.apple.notes.table');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZFILENAME) VALUES(31,5,'media','fixture.bin');")
        db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZMERGEABLEDATA1=? WHERE Z_PK=32',((fixtures/'full-table.gz').read_bytes(),))
        for pk,name in [(3,'native-plain'),(4,'native-rich'),(5,'split-title'),(6,'title-attachment'),(7,'native-indent-0'),(8,'native-indent-1'),(9,'malformed')]:
            db.execute('INSERT INTO ZICNOTEDATA(Z_PK,ZDATA) VALUES(?,?)',(pk,(fixtures/(name+'.gz')).read_bytes()))
            db.execute('INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(?,2,?,?,12,?)',(40+pk,name,'[ab-test] Synthetic',pk))
        db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZCREATIONDATE1=?,ZMODIFICATIONDATE1=? WHERE Z_PK=43',(800000000.125,800000001.5))
    media.parent.mkdir(parents=True,exist_ok=True);media.write_bytes(original)
    operations=[dict(operation='notes.delete',proven=True,reason='public_script_notestore_verified'),dict(operation='notes.delete_folder',proven=False,reason='capability_unproven'),dict(operation='notes.move',proven=True,reason='public_script_move_rename'),dict(operation='notes.rename_folder',proven=False,reason='capability_unproven')]
    coverage=dict(fields='partial',operations=operations,missing=['unknown protobuf fields and unavailable attachment bytes are explicit; rich HTML text edits require preserved structure; checklist edits remain unsupported; folder deletion requires per-build proof'])
    context=dict(observed_at='2026-05-29T00:00:00.000Z',checkpoint='synthetic-checkpoint',scan_id='synthetic-scan',coverage=coverage,facts={},fact_types=['public.data','com.apple.notes.table','com.apple.notes.inlinetextattachment.hashtag','public.png',''],fact_pairs=[dict(uti='public.data',extension='bin'),dict(uti='public.png',extension='png'),dict(uti='public.data',extension='png')])
    (root/'context.json').write_text(json.dumps(context))
elif operation=='remove-media':media.unlink()
elif operation=='restore-media':media.write_bytes(original)
elif operation=='change-media':media.write_bytes(b'[ab-test] changed attachment bytes')
elif operation=='escape-media':
    media.unlink();(root.parent/'outside.bin').write_bytes(b'synthetic outside bytes');media.symlink_to(root.parent/'outside.bin')
elif operation=='image':
    import struct,zlib
    def chunk(kind,payload):return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
    payload=b'\x00'+bytes([255,0,0,255])*2
    png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(payload*2))+chunk(b'IEND',b'')
    media.with_name('fixture.png').write_bytes(png)
    with sqlite3.connect(path) as db:
        db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFILENAME='fixture.png' WHERE Z_PK=31")
        db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTYPEUTI1='public.png' WHERE Z_PK=30")
elif operation=='repair-media':
    media.unlink();media.write_bytes(original)
else:
    with sqlite3.connect(path) as db:
        if operation=='held-title':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Held v1' WHERE Z_PK=43")
        elif operation=='held-title2':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Held v2' WHERE Z_PK=43")
        elif operation=='restart-delete':db.execute('DELETE FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=45')
        elif operation=='title':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Changed' WHERE Z_PK=43")
        elif operation=='bookkeeping':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZSERVERRECORDDATA=x'010203',ZGENERATION='cloud-generation',ZISPINNED=NULL,ZISPASSWORDPROTECTED=NULL WHERE Z_PK=43")
        elif operation=='no-revision':db.executescript("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='ignored revision' ,Z_OPT=Z_OPT+1 WHERE Z_PK=43;UPDATE ZICCLOUDSYNCINGOBJECT SET Z_OPT=Z_OPT-1 WHERE Z_PK=43;")
        elif operation=='repair-no-revision':db.executescript("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Synthetic',Z_OPT=Z_OPT+1 WHERE Z_PK=43;UPDATE ZICCLOUDSYNCINGOBJECT SET Z_OPT=Z_OPT-1 WHERE Z_PK=43;")
        elif operation=='body':db.execute('UPDATE ZICNOTEDATA SET ZDATA=? WHERE Z_PK=3',((fixtures/'native-rich.gz').read_bytes(),))
        elif operation=='body-repair':db.execute('UPDATE ZICNOTEDATA SET ZDATA=? WHERE Z_PK=3',((fixtures/'native-plain.gz').read_bytes(),))
        elif operation=='locked':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZISPASSWORDPROTECTED=1 WHERE Z_PK=43')
        elif operation=='unlocked':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZISPASSWORDPROTECTED=0 WHERE Z_PK=43')
        elif operation=='trash':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZPARENT=11 WHERE Z_PK=12')
        elif operation=='untrash':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZPARENT=10 WHERE Z_PK=12')
        elif operation=='smart':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZSMARTFOLDERQUERYJSON='{}' WHERE Z_PK=12")
        elif operation=='unsmart':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZSMARTFOLDERQUERYJSON=NULL WHERE Z_PK=12')
        elif operation=='account':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZNAME='Changed Account' WHERE Z_PK=1")
        elif operation=='orphan-attachment':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZNOTE1=43 WHERE Z_PK=30')
        elif operation=='repair-attachment':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZNOTE1=20 WHERE Z_PK=30')
        elif operation=='cycle-folder':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZPARENT=12 WHERE Z_PK=10')
        elif operation=='repair-cycle':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZPARENT=NULL WHERE Z_PK=10')
        elif operation=='rename-key':db.execute("UPDATE ZICCLOUDSYNCINGOBJECT SET ZIDENTIFIER='renamed-plain' WHERE Z_PK=43")
        elif operation=='delete-note':db.execute('DELETE FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=43')
        elif operation=='mark-note':db.execute('UPDATE ZICCLOUDSYNCINGOBJECT SET ZMARKEDFORDELETION=1 WHERE Z_PK=44')
        elif operation=='schema':db.execute('ALTER TABLE ZICCLOUDSYNCINGOBJECT ADD COLUMN ZSYNTHETIC INTEGER')
        elif operation=='uuid':db.execute("UPDATE Z_METADATA SET Z_UUID='ab-test-notes-store-2'")
        elif operation=='empty':db.executescript('DELETE FROM ZICCLOUDSYNCINGOBJECT;DELETE FROM ZICNOTEDATA;')
        elif operation=='missing-revisions':db.execute('ALTER TABLE ZICNOTEDATA RENAME COLUMN Z_OPT TO MISSING_OPT')
        elif operation=='repair-revisions':db.execute('ALTER TABLE ZICNOTEDATA RENAME COLUMN MISSING_OPT TO Z_OPT')
        else:raise ValueError(operation)
