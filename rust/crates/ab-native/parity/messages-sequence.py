import json, os, pathlib, plistlib, shutil, sqlite3, subprocess, sys, tempfile

binary, fixtures, output = sys.argv[1:]
binary = str(pathlib.Path(binary).resolve())
fixtures = pathlib.Path(fixtures).resolve()
root = pathlib.Path(tempfile.mkdtemp(prefix='ab-messages-synthetic-'))
home = root / 'home'
attachments = home / 'Library/Messages/Attachments'
attachments.mkdir(parents=True)
(attachments / 'synthetic.txt').write_text('synthetic blob')
path = root / 'chat.db'

def seed(path):
    db = sqlite3.connect(path)
    db.executescript('''PRAGMA journal_mode=WAL;
    CREATE TABLE message(guid TEXT,text TEXT,attributedBody BLOB,handle_id INTEGER,is_from_me INTEGER,service TEXT,date INTEGER,date_delivered INTEGER,date_read INTEGER,date_edited INTEGER,date_retracted INTEGER,message_summary_info BLOB,associated_message_guid TEXT,associated_message_type INTEGER,associated_message_emoji TEXT,reply_to_guid TEXT,thread_originator_guid TEXT,item_type INTEGER,group_action_type INTEGER,group_title TEXT,other_handle INTEGER,balloon_bundle_id TEXT,payload_data BLOB,expressive_send_style_id TEXT,error INTEGER,subject TEXT,schedule_type INTEGER,schedule_state INTEGER);
    CREATE TABLE handle(id TEXT,service TEXT);
    CREATE TABLE chat(guid TEXT,service_name TEXT,display_name TEXT,is_filtered INTEGER,properties BLOB);
    CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
    CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
    CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER);
    CREATE TABLE attachment(guid TEXT,filename TEXT,mime_type TEXT,uti TEXT,total_bytes INTEGER,transfer_state INTEGER,emoji_image_content_identifier TEXT,emoji_image_short_description TEXT);
    CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER);
    INSERT INTO handle VALUES('synthetic@example.invalid','iMessage');
    INSERT INTO chat VALUES('iMessage;+;synthetic','iMessage','synthetic',0,NULL);
    INSERT INTO chat_handle_join VALUES(1,1);
    ''')
    db.executemany('INSERT INTO message(guid,text,handle_id,is_from_me,service,date) VALUES(?,?,1,1,?,?)',[(f'synthetic-{i:04}',f'synthetic {i}','iMessage',800000000000000000+i*1000000000) for i in range(2010)])
    db.executemany('INSERT INTO chat_message_join VALUES(1,?)',[(i,) for i in range(1,2011)])
    db.execute('INSERT INTO attachment VALUES(?,?,?,?,?,?,?,?)',('synthetic-attachment','~/Library/Messages/Attachments/synthetic.txt','text/plain','public.plain-text',14,5,None,None))
    db.execute('INSERT INTO message_attachment_join VALUES(2010,1)')
    db.commit()
    return db

db=seed(path)
cwd=root/'synthetic-cwd'
cwd.mkdir()
process=subprocess.Popen([binary,str(path),str(home)]+(["--paged"] if os.environ.get("MESSAGES_PAGED")=="1" else []),stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,cwd=cwd)
results=[]
raw_results=[]
metrics=[]
def step(name,complete=True):
    db.commit()
    process.stdin.write('complete\n' if complete else 'change\n');process.stdin.flush()
    rows=[]
    raw_rows=[]
    while True:
        line=process.stdout.readline()
        if not line:raise RuntimeError(f'{name}: reader exited {process.poll()}')
        if line.startswith('END'):
            if line.startswith('END '):metrics.append({'stage':name,**json.loads(line[4:])})
            break
        raw_rows.append(line.strip())
        row=json.loads(line)
        rows.append(row)
    results.append({'stage':name,'observations':rows})
    raw_results.append('{"stage":'+json.dumps(name)+',"observations":['+','.join(raw_rows)+']}')

try:
    if 'messages-reader' in binary:
        for request,expected in [('synthetic-attachment 0 128',{'ok':True,'text':'synthetic blob'}),('synthetic-attachment 0 131073',{'ok':False,'text':None}),('unknown 0 128',{'ok':False,'text':None})]:
            process.stdin.write('blob '+request+'\n');process.stdin.flush()
            assert json.loads(process.stdout.readline()[5:])==expected
            assert process.stdout.readline().strip()=='END'
    step('initial')

    step('idle')
    for i in range(20):step('idle-repeat-'+str(i))
    db.execute("UPDATE message SET text='edited',date_edited=800000100000000000 WHERE guid='synthetic-2009'")
    db.execute('UPDATE message SET message_summary_info=? WHERE guid=?',(plistlib.dumps({'ec':{'0':[{'d':800000100.0,'t':(fixtures/'synthetic.typedstream').read_bytes()}]}},fmt=plistlib.FMT_BINARY),'synthetic-2009'))
    step('edit')
    db.execute("UPDATE message SET text=NULL,date_retracted=800000101000000000,message_summary_info=? WHERE guid='synthetic-2008'",(plistlib.dumps({'rp':[0]},fmt=plistlib.FMT_BINARY),))
    step('unsend')
    db.execute("INSERT INTO message(guid,associated_message_type,associated_message_guid,associated_message_emoji,date) VALUES('synthetic-reaction',2006,'p:0/synthetic-0000','🧪',800000102000000000)")
    db.execute("UPDATE message SET text='old referenced edit' WHERE guid='synthetic-0000'")
    step('reaction-and-old-reference',False)
    db.execute("UPDATE message SET associated_message_type=3006 WHERE guid='synthetic-reaction'")
    step('reaction-remove')
    db.execute('DELETE FROM chat_message_join WHERE message_id=1')
    db.execute('INSERT INTO chat_recoverable_message_join VALUES(1,1,800000103000000000)')
    step('recoverable-old')
    db.execute('DELETE FROM chat_recoverable_message_join')
    db.execute('INSERT INTO chat_message_join VALUES(1,1)')
    step('restore-old')
    db.executescript("UPDATE chat SET display_name='renamed'; INSERT INTO handle VALUES('other@example.invalid','iMessage'); INSERT INTO chat_handle_join VALUES(1,2)")
    step('membership')
    db.execute('ALTER TABLE chat ADD COLUMN chat_identifier TEXT')
    db.execute("UPDATE chat SET chat_identifier='synthetic-pinned'")
    preferences=home/'Library/Preferences'
    preferences.mkdir(parents=True)
    pins=preferences/'com.apple.messages.pinning.plist'
    pins.write_bytes(plistlib.dumps({'pD':{'pP':['synthetic-pinned']}},fmt=plistlib.FMT_BINARY))
    step('pins')
    pins.unlink()
    step('pins-removed')
    db.execute("UPDATE attachment SET transfer_state=0,filename=NULL")
    step('attachment-pending')
    db.execute("UPDATE attachment SET transfer_state=5,filename='~/Library/Messages/Attachments/offloaded.txt'")
    step('attachment-offloaded')
    db.execute("UPDATE attachment SET filename=NULL")
    step('attachment-missing')
    for i,fixture in enumerate(['synthetic.typedstream','synthetic-crlf.typedstream','synthetic-long.typedstream']):
        if (fixtures/fixture).exists():
            db.execute('UPDATE message SET attributedBody=? WHERE guid=?',((fixtures/fixture).read_bytes(),f'synthetic-{2000+i:04}'))
    db.execute("UPDATE message SET attributedBody=x'ff00' WHERE guid='synthetic-2003'")
    step('bodies')
    db.execute("UPDATE message SET item_type=1,group_action_type=0,other_handle=2 WHERE guid='synthetic-2004'")
    step('group-event')
    db.execute("UPDATE message SET date=800000000.123,schedule_type=1,schedule_state=8.5 WHERE guid='synthetic-2007'")
    step('sqlite-real-values')
    for i,fixture in enumerate(['rich-formatting.typedstream','rich-genmoji.typedstream']):
        db.execute('UPDATE message SET attributedBody=? WHERE guid=?',((fixtures/fixture).read_bytes(),f'synthetic-{2005+i:04}'))
    db.execute("INSERT INTO attachment VALUES('synthetic-genmoji','~/Library/Messages/Attachments/glyph.png','image/png','public.png',0,0,'synthetic-content','synthetic glyph')")
    db.execute('INSERT INTO message_attachment_join VALUES(2007,2)')
    step('rich-parts')
    db.execute("UPDATE attachment SET emoji_image_short_description='updated synthetic glyph' WHERE guid='synthetic-genmoji'")
    step('genmoji-metadata')
    db.execute("UPDATE message SET attributedBody=NULL WHERE guid='synthetic-2006'")
    step('remove-genmoji')

    db.execute("DELETE FROM message WHERE guid='synthetic-0001'")
    step('deletion-before-complete',False)
    assert not any(o['kind']=='absent' for o in results[-1]['observations'])
    step('deletion-complete')
    db.execute('ALTER TABLE message DROP COLUMN schedule_state')
    step('schema-gap')
    db.execute('DROP TABLE attachment')
    step('missing-table')
    db.close()
    replacement=root/'replacement.db'
    fresh=seed(replacement)
    fresh.execute("DELETE FROM message WHERE guid!='synthetic-0000'")
    fresh.commit();fresh.execute('PRAGMA wal_checkpoint(TRUNCATE)');fresh.execute('PRAGMA journal_mode=DELETE');fresh.close()
    os.replace(replacement,path)
    for suffix in ['-wal','-shm']:
        try:os.unlink(str(path)+suffix)
        except FileNotFoundError:pass
    db=sqlite3.connect(path)
    step('replacement')
    step('replacement-idle')
    if metrics:
        pathlib.Path(output+'.metrics').write_text(json.dumps(metrics,sort_keys=True))
        assert all(m['hashes']==metrics[0]['hashes'] and m['rows']==metrics[0]['rows'] for m in metrics[1:22]), 'idle did history work'
        assert metrics[-2]['generation']!=metrics[-3]['generation'], 'replacement generation unchanged'
        assert metrics[metrics.index(next(m for m in metrics if m['stage']=='schema-gap'))]['generation'] != next(m for m in metrics if m['stage']=='deletion-complete')['generation']
        pathlib.Path(output+'.metrics').write_text(json.dumps(metrics,sort_keys=True))
    pathlib.Path(output).write_text('['+','.join(raw_results)+']')
    if 'messages-reader' in binary:
        db.executescript("DELETE FROM message; UPDATE chat SET display_name='interrupt start'")
        db.commit()
        db.executemany('INSERT INTO message(guid,text) VALUES(?,?)',[(f'synthetic-interrupt-{i:04}',str(i)) for i in range(300)])
        db.commit()
        for index,request in enumerate(['begin','page','page']):
            if index==2:
                db.execute("DELETE FROM message WHERE guid='synthetic-interrupt-0000'")
                db.execute("INSERT INTO message(guid,text) VALUES('synthetic-interrupt','live insert during history')")
                db.commit()
            process.stdin.write(request+'\n');process.stdin.flush()
            while True:
                line=process.stdout.readline()
                if line.startswith('END'):
                    assert not json.loads(line[4:])['complete'], 'interrupted scan claimed completeness'
                    break
                assert json.loads(line)['kind']!='absent', 'interrupted scan emitted absence'
        process.stdin.write('resume messages.message synthetic-resume-absent\n');process.stdin.flush()
        assert process.stdout.readline().strip()=='END'
        absent=[]
        for _ in range(20):
            process.stdin.write('page\n');process.stdin.flush()
            page=[]
            while True:
                line=process.stdout.readline()
                if line.startswith('END'):
                    complete=json.loads(line[4:])['complete']
                    break
                page.append(json.loads(line))
            if not complete:assert not any(o['kind']=='absent' for o in page)
            absent.extend(o['native_key'] for o in page if o['kind']=='absent')
            if complete:break
        else:raise AssertionError('paged scan never completed')
        assert 'synthetic-resume-absent' in absent
        assert db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()[0]==0, 'reader pinned WAL between pages'
    print('synthetic stages',len(results),'observations',sum(len(s['observations']) for s in results))
finally:
    if not pathlib.Path(output).exists():pathlib.Path(output+'.partial').write_text('['+','.join(raw_results)+']')
    process.stdin.close();process.wait(timeout=10)
    db.close();shutil.rmtree(root)
