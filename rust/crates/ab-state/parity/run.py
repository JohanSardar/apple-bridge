import subprocess,json,tempfile,hashlib,pathlib,time,statistics,sqlite3,os
ROOT=pathlib.Path(__file__).resolve().parents[4]
RUST=os.environ.get('AB_STATE_RUST',str(ROOT/'rust/target/release/examples/state_probe'))
GO=os.environ.get('AB_STATE_GO','/tmp/ab-state-go')
vis={s:True for s in ['messages','contacts','reminders','calendar','system','notes','health','commands:all']}
def ident(prefix,n):return prefix+'_'+str(n).zfill(26)
def record(c,n,d,version=1):
 prefixes={'messages.chat':'cht','messages.message':'msg','messages.handle':'hdl','messages.attachment':'att','messages.reaction':'rxn','contacts.contact':'con','reminders.list':'lst','reminders.reminder':'rmd','calendar.event':'evt','notes.folder':'fld','notes.note':'nte','health.sample':'hsm','system.source':'src'}
 return dict(collection=c,id=ident(prefixes[c],n),version=version,source=c.split('.')[0],observed_at='2026-11-02T12:00:00Z',deleted=False,coverage=dict(fields='complete',missing=None),data=d)
corpus=[record('messages.chat',1,dict(display_name='Synthetic chat',participants=[ident('hdl',1)],filter='known')),record('messages.handle',1,dict(address='test@example.invalid',service='iMessage',contact_id=ident('con',1))),record('contacts.contact',1,dict(given_name='Synthetic',family_name='Person')),record('reminders.list',1,dict(title='Synthetic list'))]
for n in range(1,301):corpus.append(record('messages.message',n,dict(chat_id=ident('cht',1),sent_at=f'2026-11-02T12:{n//60:02}:{n%60:02}Z',text=f'Synthetic hello {n}',kind='message',is_from_me=n%2==0,thread_originator_id=ident('msg',1) if n<10 else '',sender_handle_id=ident('hdl',1),service='iMessage')))
for n in range(1,101):corpus.append(record('reminders.reminder',n,dict(title=f'Item {n}',list_id=ident('lst',1),completed=n%3==0,flagged=n%2==0,tags=['synthetic'],due='2026-11-02',assignment=dict(assignee=dict(id='self',is_me=True)))))
corpus.extend([record('notes.folder',1,dict(title='Synthetic folder',account='synthetic',account_name='Synthetic notes')),record('notes.note',1,dict(title='Synthetic note',folder_id=ident('fld',1),plain_text='Synthetic note\nHello note',editable='plain',modified_at='2026-11-02T12:00:00Z',created_at='2026-11-02T12:00:00Z')),record('health.sample',1,dict(type='step_count',start='2026-11-02T12:00:00Z',end='2026-11-02T12:05:00Z',value=100,unit='count')),record('system.source',1,dict(source='messages',state='ready'))])
corpus.extend([record('calendar.event',1,dict(start='2026-11-02T15:00:00Z',end='2026-11-02T16:00:00Z',title='Synthetic event')),record('calendar.event',2,dict(start='2026-11-02',end='2026-11-03',all_day=True,title='Synthetic all day'))])
corpus.append(record('messages.reaction',1,dict(message_id=ident('msg',300),kind='love',sent_at='2026-11-02T13:01:00Z',sender_handle_id=ident('hdl',1),part_index=0)))
corpus.append(record('messages.attachment',1,dict(message_id=ident('msg',1),mime_type='image/png',filename='synthetic.png',blob=dict(digest='a'*64,size=120,state='present'))))
def digest(rs):return hashlib.sha256(''.join(f'{r["collection"]}\0{r["id"]}\0{r["version"]}\n' for r in sorted(rs,key=lambda r:r['collection']+'/'+r['id'])).encode()).hexdigest()
def snapshot(rs,seq=1000):return dict(op='snapshot',epoch='synthetic-epoch',revision=1,begin=dict(id='synthetic-snapshot',s=seq,count=len(rs),digest=digest(rs),expires_at='2099-01-01T00:00:00Z'),pages=[dict(id='synthetic-snapshot',ordinal=0,records=rs)])
class Driver:
 def __init__(self,exe,home):self.impl="rust" if exe==RUST else "go";self.p=subprocess.Popen([exe,home],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
 def call(self,q):self.p.stdin.write(json.dumps({'visibility':vis,**q})+'\n');self.p.stdin.flush();raw=self.p.stdout.readline().strip();assert raw, self.p.poll();return raw
 def close(self):self.p.stdin.close();assert self.p.wait()==0
count=0;timings={};model_timings={}
with tempfile.TemporaryDirectory(prefix='ab-state-parity-') as tmp:
 homes=[tmp+'/go',tmp+'/rust'];drivers=[Driver(GO,homes[0]),Driver(RUST,homes[1])]
 def compare(q):
  global count
  results=[]
  for d in drivers:
   name=d.impl
   start=time.perf_counter();results.append(d.call(q));elapsed=(time.perf_counter()-start)*1000;timings.setdefault(name,[]).append(elapsed)
   if q['op']=='model':model_timings.setdefault(q['operation'],{}).setdefault(name,[]).append(elapsed)
  
  if results[0]!=results[1]:
   a,b=map(json.loads,results)
   if q['op']=='references':print([(home,sqlite3.connect(home+'/state.db').execute('SELECT owner,digest FROM blob_refs ORDER BY owner,digest').fetchall()) for home in homes],flush=True)
   print('DIFF',q, 'same parsed',a==b, 'first byte',next((i for i,(x,y) in enumerate(zip(*results)) if x!=y),-1),flush=True)
   raise AssertionError((results[0][-400:],results[1][-400:]))
  count+=1;return json.loads(results[0])
 compare(snapshot(corpus))
 queries=[dict(op='references'),dict(op='read',query={}),dict(op='read',query=dict(collection='messages.message',text='hello',limit=5)),dict(op='read',query=dict(collection='messages.handle')),dict(op='digest')]
 for operation,query in [('messages.chats',{}),('messages.thread',dict(chat_id=ident('cht',1),limit=20)),('messages.thread',dict(chat_id=ident('cht',1),anchor='oldest',limit=20)),('messages.thread',dict(chat_id=ident('cht',1),anchor='unread')),('messages.search',dict(text='hello',limit=20)),('messages.collection',dict(chat_id=ident('cht',1),kind='photos')),('messages.handles',dict(ids=[ident('hdl',1)])),('reminders.lists',dict(time_zone='UTC')),('reminders.items',dict(time_zone='UTC',list=ident('lst',1)))]:queries.append(dict(op='model',operation=operation,query=query))
 queries.extend([dict(op='model',operation=op,query=q) for op,q in [('contacts.lookup',dict(q='synthetic')),('contacts.list',{}),('sources.status',{}),('notes.list',{}),('health.summary',dict(period='day',date='2026-11-02',time_zone='UTC')),('calendar.range',dict(start='2026-11-02T00:00:00Z',end='2026-11-03T00:00:00Z')),('replica.status',{})]])
 queries.extend([dict(op='model',operation='calendar.agenda',query=dict(start='2026-11-02T00:00:00-06:00',end='2026-11-03T00:00:00-06:00'))])
 queries.extend([dict(op='model',operation='messages.chats',query=dict(filter=f)) for f in ['all','known','unknown','unread','spam','archived','deleted']])
 queries.extend([dict(op='model',operation='reminders.items',query=dict(time_zone='UTC',smart=f)) for f in ['today','scheduled','all','flagged','completed','assigned']])
 queries.extend([dict(op='model',operation='messages.collection',query=dict(chat_id=ident('cht',1),kind=f)) for f in ['media','audio','documents','links','locations']])
 queries.extend([dict(op='model',operation='reminders.items',query=dict(time_zone='UTC',list=ident('lst',1),sort=f)) for f in ['title','due','created','priority','manual']])
 queries.extend([dict(op='read',visibility=dict(messages=True),query=dict(collection='messages.handle')),dict(op='model',visibility=dict(messages=True),operation='messages.handles',query=dict(ids=[ident('hdl',1)])),dict(op='model',operation='messages.chats',query=dict(participants=['test@example.invalid']))])
 for q in queries:compare(q)
 for op,q in [('messages.chats',{}),('messages.search',dict(text='hello')),('messages.thread',dict(chat_id=ident('cht',1)))]:
  result=compare(dict(op='model',operation=op,query=dict(q,limit=5)))
  edge=result.get('next') or result.get('before')
  if edge:compare(dict(op='model',operation=op,query=dict(q,limit=5,**({'before':edge} if op=='messages.thread' else {'after':edge}))))
 new=record('messages.message',999,dict(chat_id=ident('cht',1),kind='message',text='atomic synthetic',sent_at='2026-11-02T13:00:00Z'))
 batch=dict(epoch='synthetic-epoch',subscription_revision=1,after=1000,through=1001,partial=True,changes=[dict(sequence=1001,op='upsert',record=new)])
 for bad in [dict(batch,epoch='wrong'),dict(batch,subscription_revision=2),dict(batch,after=999),dict(batch,after=1001),dict(batch,changes=[dict(sequence=1000,op='upsert',record=new)])]:compare(dict(op='apply',batch=bad))
 bad=snapshot(corpus);bad['begin']['digest']='bad';compare(bad);compare(dict(op='head'))
 bad=snapshot(corpus);bad['begin']['count']+=1;compare(bad);compare(dict(op='head'))
 compare(dict(op='apply',batch=batch));compare(dict(op='head'));compare(dict(op='read',query=dict(collection='messages.message')))
 compare(dict(op='apply',batch=dict(batch,after=1001,through=1002,partial=False,changes=[])));compare(dict(op='head'));compare(dict(op='digest'));compare(dict(op='changes',cursor=dict(epoch='synthetic-epoch',subscription_revision=1,sequence=1000)))
 compare(dict(op='apply',batch=dict(batch,after=1002,through=1003,partial=True,changes=[dict(sequence=1003,op='upsert',record=dict(new,id=ident('msg',998)))])))
 for d in drivers:d.close()
 drivers=[Driver(RUST,homes[0]),Driver(GO,homes[1])]
 for q in queries:compare(q)
 for q in queries:
  for repeat in range(10):compare(q)
 compare(dict(op='restrict',visibility={'messages':True,'commands:all':True}));compare(dict(op='read',query={}));
 compare(snapshot(corpus[:4],2000));compare(dict(op='read',query={}));compare(dict(op='digest'))
 for d in drivers:d.close()
 for home in homes:
  db=sqlite3.connect(home+'/state.db');assert db.execute("select value from meta where key='schema_version'").fetchone()[0]=='2';assert os.stat(home+'/state.db').st_mode&0o777==0o600;assert db.execute('pragma integrity_check').fetchone()[0]=='ok';db.close()
print(json.dumps(dict(model_timing_ms={op:{impl:dict(n=len(v),p50=statistics.median(v),p95=sorted(v)[int(.95*(len(v)-1))],max=max(v)) for impl,v in by_impl.items()} for op,by_impl in model_timings.items()},comparisons=count,mismatches=0,impl='rust',bidirectional_reopen=True,timing_ms={k:dict(n=len(v),p50=statistics.median(v),p95=sorted(v)[int(.95*(len(v)-1))],max=max(v)) for k,v in timings.items()}),indent=2))
