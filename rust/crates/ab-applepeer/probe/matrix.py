import datetime
import http.client
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import threading

class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__('localhost', timeout=10)
        self.path = path
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(self.path)

def control(path, method, url, value=None):
    conn = UnixHTTP(path)
    conn.request(method, url, None if value is None else json.dumps(value), {'Content-Type':'application/json'})
    response = conn.getresponse()
    data = response.read()
    assert response.status == 200, (url, response.status, data)
    conn.close()
    return json.loads(data)

def normalized_evidence(value):
    if isinstance(value,list):
        return [normalized_evidence(item) for item in value]
    if not isinstance(value,dict):
        return value
    generated={'native_key','capture_checkpoint','ticket','expires_at','blob_id'}
    return {key: ('<generated>' if item else item) if key in generated else normalized_evidence(item) for key,item in value.items()}

def run(peer, lab, fault, lifecycle=False, artifact=None, missing_uploads=False, multipart=False, resolve_fault=None, clock_offset=None, clock_expiry=None):
    with tempfile.TemporaryDirectory(prefix='gb-ipc-') as directory:
        home = Path(directory)
        secret = home / 'secret'
        secret.write_bytes(bytes([7]*32))
        secret.chmod(0o600)
        server = subprocess.Popen([peer, str(home/'peer.sock')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,env=dict(os.environ,APPLE_BRIDGE_HOME=str(home)))
        apple = subprocess.Popen([lab, 'simapple', '--socket', str(home/'peer.sock'), '--secret-file', str(secret), '--control', str(home/'control.sock')], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        def exchange(frame):
            server.stdin.write(json.dumps(frame)+'\n')
            server.stdin.flush()
            line = server.stdout.readline()
            assert line, (server.poll(), apple.poll(), 'probe exited')
            return json.loads(line)
        clock_worker=None
        try:
            deadline = time.monotonic()+15
            while time.monotonic() < deadline:
                if exchange({'t':'Available'})['available'] and (home/'control.sock').exists():
                    break
                time.sleep(.05)
            else:
                raise AssertionError(('not ready', apple.poll()))
            assert (home/'peer.sock').stat().st_mode & 0o777 == 0o600
            ready_deadline=time.monotonic()+10
            for attempt in range(10):
                ident=f'qry_ready_{attempt}'
                query=exchange({'t':'LocalQuery','args':{'id':ident,'op':'helper.version','args':{},'deadline_ms':1000}})
                if query.get('error',{}).get('code')!='deadline' or time.monotonic()>=ready_deadline:break
            assert query.get('id')==ident and query.get('result_json'),query
            blob=exchange({'t':'Query','id':'qry_blob','op':'test.blob','args':{'size':262145},'deadline_ms':1000})
            assert blob.get('t')=='QueryResult',blob
            ref=blob['blobs'][0]
            content=exchange({'t':'ReadBlob','handle':ref['handle'],'length':ref['size']})
            import base64
            import hashlib
            data=base64.b64decode(content.get('bytes',''))
            assert len(data)==ref['size'] and hashlib.sha256(data).hexdigest()==ref['digest'] and content.get('eof'),content
            missing=exchange({'t':'ReadBlob','handle':'missing-fixture','length':1})
            assert missing.get('error',{}).get('code')=='not_found',missing
            if not fault and not lifecycle:
                for state,code in [('offloaded','unsupported'),('missing','not_found')]:
                    control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':state})
                    unavailable=exchange({'t':'ReadBlob','handle':ref['handle'],'length':1})
                    assert unavailable.get('error',{}).get('code')==code,unavailable
                control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'present','unavailable_reads':1})
                unavailable=exchange({'t':'ReadBlob','handle':hashlib.sha256(bytes(1)).hexdigest(),'length':1})
                assert unavailable.get('error',{}).get('code')=='unavailable',unavailable
                control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'present'})
            hydrated=exchange({'t':'LocalQuery','args':{'id':'qry_audio','op':'speech.synthesize','args':{'text':'Bridge fixture'},'deadline_ms':1000}})
            assert hydrated.get('id')=='qry_audio' and len(hydrated.get('blobs',[]))==1 and 'handle' not in hydrated['blobs'][0],hydrated
            if fault:
                control(str(home/'control.sock'), 'POST', '/fault', {'name':fault})
            if lifecycle:
                who='agent:probe' if fault in ['approval','owner-rejection'] else 'device:probe'
                command={'command_id':'cmd_local','op':'reminders.create_list','args':{'title':'Synthetic lifecycle'},'wait_ms':15000}
                if fault in ['identical-manual-send','ui-interference','outcome-before-capture','delivery-failed','ghost-send','outcome-without-capture','receipt-before-outcome']:
                    command.update(op='messages.send',args={'handles':['self@example.com'],'service':'iMessage','text':'[ab-test probe] Synthetic lifecycle'})
                if multipart:
                    command.update(op='messages.send',args={'handles':['self@example.com'],'service':'iMessage','text':'[ab-test probe] Synthetic multipart','files':[{'name':'one.txt','data':'b25l'},{'name':'two.txt','data':'dHdv'}]})
                if artifact:
                    command.update(op='capture.screenshot',args={})
                    if artifact=='offloaded':control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'offloaded'})
                if missing_uploads:
                    command.update(op='messages.send',args={'handles':['self@example.com'],'service':'iMessage','files':[{'name':'synthetic.txt','data':'eA=='}]})
                    assert exchange({'t':'DisableUploads'})=={'disabled':True}
                if fault=='expiry':
                    command['expires_at']='2000-01-01T00:00:00Z'
                shifted=[]
                if clock_offset is not None:
                    command['expires_at']=clock_expiry or (datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(seconds=30)).isoformat().replace('+00:00','Z')
                    control(str(home/'control.sock'),'POST','/fault',{'name':'pause','ms':400})
                    def shift_clock():
                        until=time.monotonic()+10
                        while time.monotonic()<until:
                            with sqlite3.connect(home/'gateway-state'/'ledger.db') as db:
                                row=db.execute("SELECT payload FROM commands WHERE id='cmd_local'").fetchone()
                            if row:
                                assert control(str(home/'control.sock'),'GET','/effects').get('cmd_local',0)==0
                                (home/'clock-offset').write_text(str(clock_offset))
                                shifted.append(True)
                                return
                            time.sleep(.002)
                    clock_worker=threading.Thread(target=shift_clock)
                    clock_worker.start()
                submitted=exchange({'t':'Submit','principal':who,'args':command})
                if clock_worker:
                    clock_worker.join(timeout=11)
                    (home/'clock-offset').write_text('0')
                    assert shifted==[True] and submitted.get('state')=='observed',submitted

                if missing_uploads:
                    assert submitted.get('error',{}).get('code')=='unavailable',submitted
                    status=exchange({'t':'Status','principal':who,'command_id':'cmd_local'})
                    assert status.get('error',{}).get('code')=='not_found',status
                    assert not (home/'dispatch-guard.log').exists()
                    return {'error':'unavailable','admitted':False,'dispatch_count':0}
                if fault=='expiry':
                    assert submitted['error']['code']=='expired',submitted
                    assert control(str(home/'control.sock'),'GET','/effects').get('cmd_local',0)==0
                    return {'error':'expired','effect_count':0}
                assert 'error' not in submitted,submitted
                if fault in ['approval','owner-rejection']:
                    assert submitted['state']=='awaiting_approval',submitted
                    assert control(str(home/'control.sock'),'GET','/effects').get('cmd_local',0)==0
                    exchange({'t':'Decide','command_id':'cmd_local','code':'approve' if fault=='approval' else 'reject'})
                if fault in ['crash-after-effect','crash-before-outcome']:
                    apple.wait(timeout=5)
                    apple=subprocess.Popen([lab,'simapple','--socket',str(home/'peer.sock'),'--secret-file',str(secret),'--control',str(home/'control.sock')],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
                    time.sleep(.3)
                status_request={'t':'Status','principal':who,'command_id':'cmd_local'}
                for _ in range(250):
                    row=exchange(status_request)
                    if row.get('state') not in ['admitted','prepared','dispatched','awaiting_approval']:
                        break
                    time.sleep(.05)
                assert row.get('state') not in ['admitted','prepared','dispatched','awaiting_approval'],row
                if artifact=='present':
                    assert row.get('state')=='observed' and 'handle' not in row['evidence'] and row['evidence']['blob_id'].startswith('qry_'),row
                    checked=exchange({'t':'LocalQuery','args':{'id':'qry_artifact_input','op':'vision.ocr','args':{'blob':{'query_id':row['evidence']['blob_id'],'digest':row['evidence']['digest']}}}})
                    assert checked.get('error',{}).get('code')=='invalid',checked
                if artifact=='offloaded':assert row.get('state')=='indeterminate',row
                guard_path=home/'dispatch-guard.log'
                dispatches=lambda:sum(1 for line in guard_path.read_text().splitlines() if json.loads(line).get('command_id')=='cmd_local' and not json.loads(line).get('outcome')) if guard_path.exists() else 0
                dispatch_count=dispatches()
                before=control(str(home/'control.sock'),'GET','/effects')
                repeated=exchange({'t':'Submit','principal':who,'args':command})
                assert repeated.get('state')==row.get('state'),(row,repeated)
                exchange({'t':'Recover'})
                assert control(str(home/'control.sock'),'GET','/effects')==before
                if resolve_fault:control(str(home/'control.sock'),'POST','/fault',{'name':resolve_fault})
                resolved=exchange(dict(status_request,code='resolve'))
                assert control(str(home/'control.sock'),'GET','/effects')==before
                with sqlite3.connect(home/'gateway-state'/'ledger.db') as db:
                    payload=json.loads(db.execute("SELECT payload FROM commands WHERE id='cmd_local'").fetchone()[0])
                    payload['expires_at']='2000-01-01T00:00:00Z'
                    db.execute("UPDATE commands SET payload=? WHERE id='cmd_local'",(json.dumps(payload).encode(),))
                after_expiry=exchange({'t':'Submit','principal':who,'args':command})
                assert after_expiry.get('state')==resolved.get('state') and after_expiry.get('expires_at')=='2000-01-01T00:00:00Z',(after_expiry,resolved)
                old_era=row['admission_era']
                with sqlite3.connect(home/'gateway-state'/'ledger.db') as db:
                    db.execute("UPDATE meta SET value=? WHERE key='admission_era'",('synthetic-rotated-era',))
                after_era=exchange({'t':'Submit','principal':who,'args':command})
                assert after_era.get('state')==resolved.get('state'),(after_era,resolved)
                stale=exchange({'t':'Submit','principal':who,'args':dict(command,command_id='cmd_old_era',admission_era=old_era)})
                assert stale.get('error',{}).get('code')=='expired',stale
                changed=dict(command,args=dict(command['args'],text='[ab-test changed]') if command['op']=='messages.send' else {'display':1} if artifact else {'title':'Synthetic changed'})
                reused=exchange({'t':'Submit','principal':who,'args':changed})
                assert reused.get('error',{}).get('code')=='id_reused',reused
                era_resolved=exchange(dict(status_request,code='resolve'))
                assert era_resolved.get('state')==after_era.get('state'),(after_era,era_resolved)
                assert control(str(home/'control.sock'),'GET','/effects')==before
                assert dispatches()==dispatch_count
                return {'clock_jump_ms':clock_offset,'dispatch_count':dispatch_count,'old_expiry_retry':'stored','old_era_retry':'stored','stale_era_admission':'expired','state':row['state'],'disposition':row.get('disposition'),'resolved_state':resolved.get('state'),'parts':[{'index':p['index'],'disposition':p['disposition'],'record_id':bool(p.get('record_id'))} for p in row.get('parts',[])],'resolved_parts':[{'index':p['index'],'disposition':p['disposition'],'record_id':bool(p.get('record_id'))} for p in resolved.get('parts',[])],'resolved_evidence':normalized_evidence(resolved.get('evidence')),'effect_count':before.get('cmd_local',0),'observed_ids':bool(row.get('observed_record_ids')),'evidence':normalized_evidence(row.get('evidence'))}
            messages = fault in ['identical-manual-send', 'ui-interference', 'ui-interference-before', 'outcome-before-capture']
            action = {'op':'reminders.create_list','args':{'title':'Synthetic list'}}
            if messages:
                action = {'op':'messages.send','args':{},'messages':{'handles':['self@example.com'], 'text':'[ab-test probe] Synthetic text'}}
            expected_state = None
            if fault == 'conflict':
                control(str(home/'control.sock'), 'POST', '/external-change', {'key':'list-probe','collection':'reminders.reminder','data':{'title':'Synthetic original','completed':False}})
                action = {'op':'reminders.update','args':{'title':'Synthetic changed'},'native_key':'list-probe'}
                expected_state = {'title':'mismatched expected state'}
            expiry=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(seconds=.15 if fault == 'expiry' else 60)).isoformat().replace('+00:00','Z')
            request={'t':'Prepare','command_id':'cmd_probe','principal':'token:probe','request_hash':'fingerprint','expires_at':expiry,'action':action}
            if expected_state:
                request['expected_state']=expected_state
            prepared=exchange(request)
            if fault in ['conflict','ui-interference-before']:
                assert prepared['t']=='Rejected' and prepared['error']['code']=='conflict', prepared
                assert control(str(home/'control.sock'),'GET','/effects').get('cmd_probe',0)==0
                return {'disposition':prepared['disposition'], 'error':prepared['error']['code'], 'effect_count':0}
            assert prepared['t']=='Prepared', prepared
            if fault=='expiry':
                time.sleep(.2)
            outcome=exchange({'t':'StaleDispatch' if fault=='stale-fence' else 'Dispatch','command_id':'cmd_probe','ticket':prepared['ticket']})
            if fault in ['crash-after-effect','crash-before-outcome']:
                assert 'error' in outcome, outcome
                apple.wait(timeout=5)
                apple = subprocess.Popen([lab, 'simapple', '--socket', str(home/'peer.sock'), '--secret-file', str(secret), '--control', str(home/'control.sock')], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
                time.sleep(.3)
                deadline=time.monotonic()+10
                while not exchange({'t':'Available'})['available'] and time.monotonic()<deadline:
                    time.sleep(.1)
                outcome=exchange({'t':'Resolve','command_id':'cmd_probe'})
                deadline=time.monotonic()+2
                while not outcome.get('evidence') and time.monotonic()<deadline:
                    time.sleep(.02)
                    outcome=exchange({'t':'Resolve','command_id':'cmd_probe'})
            effects=control(str(home/'control.sock'), 'GET', '/effects')
            count=0 if fault in ['expiry','ui-interference','stale-fence'] else 1
            assert effects.get('cmd_probe',0)==count, effects
            if fault in ['expiry','stale-fence']:
                assert outcome.get('t')=='Rejected', outcome
                return {'disposition':outcome['disposition'], 'effect_count':0}
            resolved=exchange({'t':'Resolve','command_id':'cmd_probe'})
            assert control(str(home/'control.sock'), 'GET', '/effects')==effects
            evidence=outcome.get('evidence',{})
            assert outcome.get('t')=='Outcome', outcome
            if fault!='ui-interference':
                assert evidence.get('fingerprint')=='fingerprint', outcome
            return {'disposition':outcome.get('disposition'), 'observed':evidence.get('observed'), 'ambiguous':evidence.get('ambiguous',False), 'candidate_count':evidence.get('candidate_count',0), 'resolved_disposition':resolved.get('disposition'), 'effect_count':count, 'evidence':normalized_evidence(evidence)}
        finally:
            if clock_worker:clock_worker.join(timeout=11)
            apple.terminate()
            server.stdin.close()
            try: apple.wait(timeout=5)
            except subprocess.TimeoutExpired: apple.kill(); apple.wait()
            try: server.wait(timeout=5)
            except subprocess.TimeoutExpired: server.kill(); server.wait()

if __name__=='__main__':
    go, rust, lab = sys.argv[1:]
    for fault in ['', 'ambiguous-outcome', 'crash-after-effect', 'identical-manual-send', 'ui-interference', 'ui-interference-before', 'outcome-before-capture', 'expiry', 'conflict']:
        expected=run(go,lab,fault)
        actual=run(rust,lab,fault)
        assert actual==expected, (fault,expected,actual)
        print(json.dumps({'scenario':fault or 'ordinary','go':expected,'rust':actual,'match':True}),flush=True)

    stale = run(rust,lab,'stale-fence')
    assert stale == {'disposition':'rejected_before_dispatch', 'effect_count':0}, stale
    print(json.dumps({'scenario':'stale-fence', 'rust':stale, 'helper_rejected':True}), flush=True)

    for fault in ['', 'ambiguous-outcome', 'crash-after-effect', 'crash-before-outcome', 'identical-manual-send', 'ui-interference', 'outcome-before-capture', 'delivery-failed', 'ghost-send', 'outcome-without-capture', 'receipt-before-outcome', 'expiry', 'approval', 'owner-rejection']:
        expected=run(go,lab,fault,lifecycle=True)
        actual=run(rust,lab,fault,lifecycle=True)
        assert actual==expected, (fault,expected,actual)
        print(json.dumps({'lifecycle':fault or 'ordinary','go':expected,'rust':actual,'match':True}),flush=True)

    for artifact in ['present','offloaded']:
        expected=run(go,lab,'',lifecycle=True,artifact=artifact)
        actual=run(rust,lab,'',lifecycle=True,artifact=artifact)
        assert actual==expected,(artifact,expected,actual)
        print(json.dumps({'artifact':artifact,'go':expected,'rust':actual,'match':True}),flush=True)

    expected=run(go,lab,'',lifecycle=True,missing_uploads=True)
    actual=run(rust,lab,'',lifecycle=True,missing_uploads=True)
    assert actual==expected,(expected,actual)
    print(json.dumps({'missing_uploads':True,'go':expected,'rust':actual,'match':True}),flush=True)

    for fault,resolve_fault in [(f,None) for f in ['', 'part-no-effect','part-progress','part-progress-failure','part-delivery-failed']] + [('part-progress','part-progress-stale')] + [('',f) for f in ['part-receipt-failed','part-receipt-failed-first','part-unbound-failure']]:
        expected=run(go,lab,fault,lifecycle=True,multipart=True,resolve_fault=resolve_fault)
        actual=run(rust,lab,fault,lifecycle=True,multipart=True,resolve_fault=resolve_fault)
        assert actual==expected,(fault,resolve_fault,expected,actual)
        print(json.dumps({'multipart':fault or 'ordinary','resolve_fault':resolve_fault,'go':expected,'rust':actual,'match':True}),flush=True)

    for offset in [3600000,-3600000]:
        expiry=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(seconds=30)).isoformat().replace('+00:00','Z')
        expected=run(go,lab,'',lifecycle=True,clock_offset=offset,clock_expiry=expiry)
        actual=run(rust,lab,'',lifecycle=True,clock_offset=offset,clock_expiry=expiry)
        assert actual==expected,(offset,expected,actual)
        print(json.dumps({'clock_jump_ms':offset,'go':expected,'rust':actual,'match':True}),flush=True)
