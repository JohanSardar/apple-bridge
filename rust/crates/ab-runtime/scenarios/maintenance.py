import json, os, pathlib, sqlite3, subprocess, tempfile, time, sys
binary=sys.argv[1]
go=sys.argv[2]
env=dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file')
checks=[]
def run(*args,ok=True):
    p=subprocess.run([binary,*args],env=env,capture_output=True,text=True,timeout=15)
    assert (p.returncode==0)==ok,(args,p.returncode,p.stderr)
    return p.stdout
with tempfile.TemporaryDirectory(prefix='ab-gf-cli-') as root:
    home=pathlib.Path(root)/'home'
    home.mkdir(mode=0o700)
    proc=subprocess.Popen([go,'start','--role','gateway','--home',str(home),'--socket',str(home/'ipc.sock'),'--api-addr','127.0.0.1:0','--peer-addr','127.0.0.1:0'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            if (home/'runtime.json').exists(): break
            assert proc.poll() is None
            time.sleep(.05)
        assert (home/'runtime.json').exists()
        run('restore','--home',str(home),'--accept-ledger-loss',ok=False)
        run('backup',str(pathlib.Path(root)/'locked.db'),'--home',str(home),ok=False)
        checks.append('Go singleton lock rejects Rust backup and restore')
    finally:
        proc.terminate(); proc.wait(timeout=10)
    ledger_inode=(home/'ledger.db').stat().st_ino
    def ledger_rows():
        with sqlite3.connect(home/'ledger.db') as db:
            return list(db.execute('SELECT * FROM commands'))
    ledger=ledger_rows()
    def cursor():
        with sqlite3.connect(home/'state.db') as db:
            return json.loads(db.execute("select value from meta where key='cursor'").fetchone()[0])
    before=cursor()
    backup=pathlib.Path(root)/'backup.db'
    run('backup',str(backup),'--home',str(home))
    run('restore',str(backup),'--home',str(home),'--accept-ledger-loss=false')
    assert cursor()['epoch']!=before['epoch']
    assert ledger_rows()==ledger
    assert (home/'ledger.db').stat().st_ino==ledger_inode
    checks.append('backup/restore rotates epoch and preserves live ledger file and command rows')
    assert home.stat().st_mode & 0o777==0o700
    assert backup.stat().st_mode & 0o777==0o600
    checks.append('home and backup retain private modes')
    def request(*args):
        try:
            subprocess.run([binary,'remote',*args,'--home',str(home)],env=env,capture_output=True,text=True,timeout=2)
        except subprocess.TimeoutExpired:
            pass
    def selections():
        return json.loads((home/'remote/ingresses.json').read_text())
    request('on','--via','both')
    assert selections()=={'relay':True,'system':True}
    request('off','--via','relay')
    assert selections()=={'relay':False,'system':True}
    document=json.loads(run('remote','status','--home',str(home)))
    assert [key for key in document if key!='message']==['id','state','backend','url','last_error','pending_link','tls_termination','certificate_expiry']
    run('remote','on','--via','invalid','--home',str(home),ok=False)
    checks.append('remote both/on/off/status and invalid backend')
    assert not json.loads((home/'remote/config.json').read_text())['backend']=='both'
    checks.append('remote selection keeps Go-readable legacy backend')
    agent=json.loads(run('agent','create','--home',str(home),'--name','fixture','--scope','messages:read','--level','read'))
    assert agent['token'] and agent['agent']['id'].startswith('agent:')
    agents=json.loads(run('agent','list','--home',str(home)))
    assert any(v['id']==agent['agent']['id'] for v in agents)
    legacy=subprocess.run([go,'agent','list','--home',str(home)],env=env,capture_output=True,text=True,timeout=15)
    assert legacy.returncode==0
    assert any(v['id']==agent['agent']['id'] for v in json.loads(legacy.stdout))
    run('agent','revoke',agent['agent']['id'],'--home',str(home))
    checks.append('agent create/list/revoke and Go credential interoperability')
    assert run('pair','--home',str(home)).strip().startswith('ab1-')
    run('pair','--home',str(home),'--iphone',ok=False)
    run('pair','--home',str(home),'--iphone','--via-remote',ok=False)
    checks.append('direct pairing and HTTPS-only phone pairing admission')
    token=run('token','create','--home',str(home),'--remote=false').strip()
    assert token
    checks.append('token create accepts explicit false boolean')
print(json.dumps({'checks':checks,'passed':len(checks)},indent=2))
