import copy, json, os, pathlib, shutil, sqlite3, subprocess, tempfile, time
ROOT=pathlib.Path(__file__).resolve().parents[4]
GO=os.environ.get('AB_GATEWAY_GO','/tmp/ab-ga-go')
RUST=os.environ.get('AB_GATEWAY_RUST',str(ROOT/'rust/target/debug/examples/gateway_probe'))
VIS={s:True for s in ['messages','contacts','calendar','notes','system','reminders','commands:all']}
class Driver:
 def __init__(self,exe,home):self.exe=exe;self.home=home;self.p=subprocess.Popen([exe,str(home)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
 def call(self,q):
  self.p.stdin.write(json.dumps(dict(visibility=VIS,**q))+'\n');self.p.stdin.flush();raw=self.p.stdout.readline();assert raw,(self.exe,self.p.poll());return json.loads(raw)
 def close(self):self.p.stdin.close();assert self.p.wait()==0
count=0
with tempfile.TemporaryDirectory(prefix='ab-gateway-parity-') as tmp:
 base=pathlib.Path(tmp)/'base';d=Driver(GO,base);head=d.call(dict(op='head'));era=d.call(dict(op='era'));d.close()
 captures=[json.loads(f['json']) for f in json.loads((ROOT/'contracts/golden/ipc.json').read_text())['frames'] if json.loads(f['json']).get('t')=='Capture']
 def cap(source,cp,pred,obs,generation='fixture',**kw):return dict(source=source,generation=generation,checkpoint=cp,predecessor=pred,observations=obs,**kw)
 def obs(c,key,data=None,kind='upsert',at='2026-01-02T03:04:05.123456789Z',**kw):return dict(collection=c,native_key=key,kind=kind,observed_at=at,**({'data':data} if data is not None else {}),**kw)
 extra=[cap('messages','1','',[obs('messages.chat','chat',dict(title='<>& snow 雪')),obs('messages.message','message',dict(chat_id={'$ref':dict(collection='messages.chat',native_key='chat')},text='one',large=9007199254740993)),obs('messages.handle','handle',dict(address='synthetic@example.invalid'))]),cap('notes','1','',[obs('notes.note','note',dict(title='note',plain_text='one',native_fingerprint='same'))]),cap('calendar','1','',[obs('calendar.event','event',dict(title='event',start='2026-01-02T00:00:00Z'))])]
 captures+=extra
 db=sqlite3.connect(base/'state.db');n=0
 for c in captures:
  for o in c['observations']:
   n+=1;prefix={'messages.chat':'cht','messages.message':'msg','messages.handle':'hdl','contacts.contact':'con','notes.note':'nte','calendar.event':'evt'}[o['collection']];rid=prefix+'_'+str(n).zfill(26)
   db.execute('INSERT OR IGNORE INTO aliases VALUES(?,?,?,?,?,?)',(c['source'],c['generation'],o['collection'],o['native_key'],o.get('scoped_alias',''),rid))
 db.execute("UPDATE aliases SET scoped_alias='stable-message' WHERE collection='messages.message'")
 db.commit();db.close()
 homes=[pathlib.Path(tmp)/'go',pathlib.Path(tmp)/'rust']
 for home in homes:shutil.copytree(base,home)
 drivers=[Driver(GO,homes[0]),Driver(RUST,homes[1])]
 def compare(q,events=False):
  global count
  results=[d.call(q) for d in drivers]
  if events:
   for r in results:
    for event in r.get('history',[]):event['at']='CLOCK'
  assert results[0]==results[1],(q,results)
  count+=1;return results[0]
 def tables(ledger=False):
  names=['commands','dedupe'] if ledger else ['records','journal','aliases','checkpoints','journal_commits']
  for table in names:
   values=[]
   for home in homes:
    db=sqlite3.connect(home/('ledger.db' if ledger else 'state.db'));cols=[r[1] for r in db.execute(f'pragma table_info({table})')];rows=db.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall();db.close()
    if table in ['records','journal']:
     normalized=[]
     for row in rows:
      row=list(row);ix=cols.index('payload');payload=row[ix];v=json.loads(payload);record=v if table=='records' else v['record']
      if record['collection']=='system.command':
       payload=payload.replace(json.dumps(record['observed_at']).encode(),b'"CLOCK"');row[ix]=payload
       if table=='records':row[cols.index('observed_at')]='CLOCK'
      normalized.append(tuple(v for k,v in zip(cols,row) if k!='committed_at'))
     rows=normalized
    if table=='commands':
     normalized=[]
     for row in rows:
      payload=row[-1]
      for event in json.loads(payload)['history']:
       payload=payload.replace(json.dumps(event['at']).encode(),b'"CLOCK"')
      normalized.append((*row[:-1],payload))
     rows=normalized
    values.append(rows)
   assert values[0]==values[1],(table,values)
 for c in captures:compare(dict(op='capture',capture=c));tables()
 compare(dict(op='capture',capture=captures[0]));tables()
 compare(dict(op='capture',capture=cap('messages','2','wrong',[])));tables()
 compare(dict(op='capture',capture=cap('messages','2','1',[obs('messages.message','message',dict(text='one',large=9007199254740993,chat_id={'$ref':dict(collection='messages.chat',native_key='chat')}),at='2026-01-02T04:00:00Z')])));tables()
 compare(dict(op='capture',capture=cap('messages','3','2',[obs('messages.message','message',dict(text='stale'),at='2026-01-02T03:00:00Z')])));tables()
 compare(dict(op='capture',capture=cap('messages','4','3',[obs('messages.message','message',kind='absent')])));tables()
 compare(dict(op='capture',capture=cap('messages','5','4',[obs('messages.message','message',kind='absent',at='2026-01-02T05:00:00Z',revalidated=True)],scan_id='scan',scope=['messages.message'],scan_started_at='2026-01-02T04:30:00Z',completion_evidence='complete')));tables()
 compare(dict(op='capture',capture=cap('calendar','2','1',[obs('calendar.event','event',kind='evict',revalidated=True)],scan_id='scan',scope=['calendar.event'],scan_started_at='2026-01-02T04:30:00Z',completion_evidence='complete')));tables()
 compare(dict(op='capture',capture=cap('notes','2','1',[obs('notes.note','note',dict(title='note',plain_text='one',native_fingerprint='same',preview='changed'))])));tables()
 compare(dict(op='capture',capture=cap('messages','6','5',[obs('messages.message','message',dict(text='resurrect'),at='2026-01-02T06:00:00Z')])));tables()
 snapshots=[d.call(dict(op='freeze',revision=1)) for d in drivers]
 for snapshot in snapshots:
  assert snapshot['digest']==snapshots[0]['digest'] and snapshot['count']==snapshots[0]['count']
 pinned=snapshots[0]['cursor']['sequence']
 compare(dict(op='capture',capture=cap('messages','7','6',[obs('messages.message','message',dict(text='after snapshot'),at='2026-01-02T07:00:00Z')])));tables()
 for home in homes:
  db=sqlite3.connect(home/'state.db');db.execute('UPDATE journal SET committed_at=0');db.commit();db.close()
 compare(dict(op='prune',rows=0));tables()
 for home in homes:
  db=sqlite3.connect(home/'state.db');assert int(db.execute("SELECT value FROM meta WHERE key='retention_floor'").fetchone()[0])==pinned;db.close()
 for d,snapshot in zip(drivers,snapshots):d.call(dict(op='renew',id=snapshot['id']));d.call(dict(op='close',id=snapshot['id']))
 compare(dict(op='prune',rows=0));tables()
 compare(dict(op='capture',capture=cap('messages','1','',[obs('messages.message','renamed',kind='alias',scoped_alias='stable-message')],generation='next-generation')));tables()
 bad=cap('messages','2','1',[obs('messages.message','rollback-new',dict(chat_id={'$ref':dict(collection='messages.chat',native_key='missing')}))],generation='next-generation')
 results=[d.call(dict(op='capture',capture=bad)) for d in drivers]
 assert all('error' in r for r in results),results
 tables()
 compare(dict(op='fence'));compare(dict(op='checkpoints'))
 for d in drivers:d.close()
 drivers=[Driver(RUST,homes[0]),Driver(GO,homes[1])]
 compare(dict(op='head'));tables()
 record=dict(command=dict(command_id='parity-command',op='reminders.create',args={},expires_at='0001-01-01T00:00:00Z'),principal='token:synthetic',source='reminders',request_hash='synthetic-hash',state='admitted',admission_era=era,created_at='2026-01-02T00:00:00Z',expires_at='2099-01-01T00:00:00Z',commit_seq=0,cursor=head,history=[])
 for state,prior in [('admitted',None),('prepared','admitted'),('dispatched','prepared'),('indeterminate','dispatched')]:
  record['state']=state;compare(dict(op='transition',record=record,**({'from':prior} if prior else {})),events=True);tables(ledger=True);tables()
 compare(dict(op='transition',record=record,**{'from':'prepared'}));
 record['request_hash']='different';compare(dict(op='transition',record=record,**{'from':'dispatched'}));record['request_hash']='synthetic-hash'
 record['state']='observed';record['commit_seq']=compare(dict(op='head'))['sequence']
 compare(dict(op='transition',record=record,**{'from':'indeterminate'}),events=True);tables(ledger=True);tables()
 record['state']='delivered';compare(dict(op='transition',record=record,**{'from':'observed'}),events=True);tables(ledger=True);tables()
 for n,states in enumerate([['awaiting_approval','rejected_by_owner'],['awaiting_approval','admitted','prepared','expired'],['admitted','rejected'],['admitted','prepared','dispatched','failed']]):
  trial=copy.deepcopy(record);trial['command']['command_id']='trial-'+str(n);trial['commit_seq']=0
  for i,state in enumerate(states):
   trial['state']=state;compare(dict(op='transition',record=trial,**({'from':states[i-1]} if i else {})),events=True);tables(ledger=True);tables()
 trial=copy.deepcopy(record);trial['command']['command_id']='parts';trial['command']['op']='messages.send';trial['source']='messages';trial['parts']=[dict(index=0,kind='text',disposition='indeterminate')];trial['commit_seq']=0
 for state,prior in [('admitted',None),('prepared','admitted'),('dispatched','prepared'),('indeterminate','dispatched')]:
  trial['state']=state;compare(dict(op='transition',record=trial,**({'from':prior} if prior else {})),events=True);tables(ledger=True);tables()
 trial['parts'][0]['disposition']='observed';trial['parts'][0]['record_id']='msg_'+str(3).zfill(26)
 compare(dict(op='parts',record=trial),events=True);tables(ledger=True);tables()
 compare(dict(op='parts',record=trial));tables(ledger=True);tables()
 trial=copy.deepcopy(record);trial['command']['command_id']='runner';trial['command']['op']='shortcuts.run';trial['source']='shortcuts';trial['commit_seq']=0
 for state,prior in [('admitted',None),('prepared','admitted'),('dispatched','prepared')]:
  trial['state']=state;compare(dict(op='transition',record=trial,**({'from':prior} if prior else {})),events=True);tables(ledger=True);tables()
 trial['state']='observed';compare(dict(op='transition',record=trial,**{'from':'dispatched'}));tables(ledger=True);tables()
 trial['evidence']=dict(exit_status=0,timed_out=False,output_base64='',effect_scope='runner completion only',prepared=dict(request_hash=trial['request_hash']))
 compare(dict(op='transition',record=trial,**{'from':'dispatched'}),events=True);tables(ledger=True);tables()
 trial=copy.deepcopy(record);trial['command']['command_id']='expired-era';trial['admission_era']='wrong';trial['state']='admitted'
 compare(dict(op='transition',record=trial));tables(ledger=True);tables()
 for d in drivers:d.close()
 tables(ledger=True)
 drivers=[Driver(GO,homes[0]),Driver(RUST,homes[1])]
 saved=[d.call(dict(op='command',id='parity-command')) for d in drivers]
 ledger_before=[sqlite3.connect(h/'ledger.db').execute('SELECT * FROM commands ORDER BY id').fetchall() for h in homes]
 for i,d in enumerate(drivers):d.call(dict(op='backup',path=str(homes[i]/'backup.db')))
 for d in drivers:d.close()
 for exe,home in zip([RUST,GO],homes):subprocess.run([exe,str(home),str(home/'backup.db')],check=True)
 drivers=[Driver(GO,homes[0]),Driver(RUST,homes[1])]
 for index,d in enumerate(drivers):
  assert d.call(dict(op='era'))!=era
  assert d.call(dict(op='head'))['epoch']!=head['epoch']
  assert d.call(dict(op='command',id='parity-command'))==saved[index]
  assert sqlite3.connect(d.home/'ledger.db').execute('SELECT * FROM commands ORDER BY id').fetchall()==ledger_before[index]
  assert d.call(dict(op='head'))['sequence']==0
 for d in drivers:d.close()
print(json.dumps(dict(impl='rust',comparisons=count,capture_payload_bytes_equal=True,journal_clock_excluded=True,command_event_clocks_normalized=True,bidirectional_reopen=True,restore_preserves_ledger=True,restore_rotates_epoch_and_era=True)))

with tempfile.TemporaryDirectory(prefix='ab-gateway-full-') as tmp:
 d=Driver(RUST,pathlib.Path(tmp));d.call(dict(op='source',source='messages'))
 before=d.call(dict(op='head'));d.call(dict(op='max_pages',pages=1))
 failed=d.call(dict(op='capture',capture=cap('messages','full','',[obs('messages.message','full',dict(text='x'*200000))])))
 assert 'full' in failed.get('error','').lower(),failed
 assert d.call(dict(op='head'))==before
 status=d.call(dict(op='model',operation='sources.status',query={}))
 assert status['records'][0]['data']['reason']=='capacity',status
 d.call(dict(op='max_pages',pages=1000000))
 recovered=d.call(dict(op='capture',capture=cap('messages','recovered','',[obs('messages.message','small',dict(text='recovered'))])))
 assert 'error' not in recovered,recovered
 status=d.call(dict(op='model',operation='sources.status',query={}))
 assert status['records'][0]['data']['state']=='ready',status
 d.close()
print(json.dumps(dict(sqlite_full_rollback=True,capacity_status=True,recovery=True)))

with tempfile.TemporaryDirectory(prefix='ab-gateway-rust-home-') as tmp:
 home=pathlib.Path(tmp)
 d=Driver(RUST,home);d.call(dict(op='capture',capture=cap('messages','fresh','',[obs('messages.message','fresh',dict(text='fresh home'))])));before=d.call(dict(op='head'));d.close()
 d=Driver(GO,home);assert d.call(dict(op='head'))==before;assert d.call(dict(op='read',query={'collection':'messages.message'}))['records'][0]['data']['text']=='fresh home';d.close()
 (home/'ledger.db').unlink()
 db=sqlite3.connect(home/'state.db');db.execute("INSERT INTO meta VALUES('commands_seen','1')");db.commit();db.close()
 result=subprocess.run([RUST,str(home)],input='',capture_output=True,text=True)
 assert result.returncode!=0 and 'LedgerMissing' in result.stderr,result
print(json.dumps(dict(rust_created_gateway_reopens_in_go=True,missing_ledger_refused=True)))
