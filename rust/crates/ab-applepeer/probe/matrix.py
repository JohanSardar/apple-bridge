import datetime
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

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
    generated={'native_key','capture_checkpoint','ticket','expires_at'}
    return {key: ('<generated>' if item else item) if key in generated else normalized_evidence(item) for key,item in value.items()}

def run(peer, lab, fault, lifecycle=False):
    with tempfile.TemporaryDirectory(prefix='gb-ipc-') as directory:
        home = Path(directory)
        secret = home / 'secret'
        secret.write_bytes(bytes([7]*32))
        secret.chmod(0o600)
        server = subprocess.Popen([peer, str(home/'peer.sock')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        apple = subprocess.Popen([lab, 'simapple', '--socket', str(home/'peer.sock'), '--secret-file', str(secret), '--control', str(home/'control.sock')], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        def exchange(frame):
            server.stdin.write(json.dumps(frame)+'\n')
            server.stdin.flush()
            line = server.stdout.readline()
            assert line, (server.poll(), apple.poll(), 'probe exited')
            return json.loads(line)
        try:
            deadline = time.monotonic()+15
            while time.monotonic() < deadline:
                if exchange({'t':'Available'})['available'] and (home/'control.sock').exists():
                    break
                time.sleep(.05)
            else:
                raise AssertionError(('not ready', apple.poll()))
            assert (home/'peer.sock').stat().st_mode & 0o777 == 0o600
            query=exchange({'t':'LocalQuery','args':{'id':'qry_probe','op':'helper.version','args':{},'deadline_ms':1000}})
            assert query.get('id')=='qry_probe' and query.get('result_json'),query
            if fault:
                control(str(home/'control.sock'), 'POST', '/fault', {'name':fault})
            if lifecycle:
                who='agent:probe' if fault in ['approval','owner-rejection'] else 'device:probe'
                command={'command_id':'cmd_local','op':'reminders.create_list','args':{'title':'Synthetic lifecycle'},'wait_ms':15000}
                if fault in ['identical-manual-send','ui-interference','outcome-before-capture']:
                    command.update(op='messages.send',args={'handles':['self@example.com'],'service':'iMessage','text':'[ab-test probe] Synthetic lifecycle'})
                if fault=='expiry':
                    command['expires_at']='2000-01-01T00:00:00Z'
                submitted=exchange({'t':'Submit','principal':who,'args':command})
                if fault=='expiry':
                    assert submitted['error']['code']=='expired',submitted
                    assert control(str(home/'control.sock'),'GET','/effects').get('cmd_local',0)==0
                    return {'error':'expired','effect_count':0}
                assert 'error' not in submitted,submitted
                if fault in ['approval','owner-rejection']:
                    assert submitted['state']=='awaiting_approval',submitted
                    assert control(str(home/'control.sock'),'GET','/effects').get('cmd_local',0)==0
                    exchange({'t':'Decide','command_id':'cmd_local','code':'approve' if fault=='approval' else 'reject'})
                if fault=='crash-after-effect':
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
                before=control(str(home/'control.sock'),'GET','/effects')
                repeated=exchange({'t':'Submit','principal':who,'args':command})
                assert repeated.get('state')==row.get('state'),(row,repeated)
                exchange({'t':'Recover'})
                assert control(str(home/'control.sock'),'GET','/effects')==before
                resolved=exchange(dict(status_request,code='resolve'))
                assert control(str(home/'control.sock'),'GET','/effects')==before
                return {'state':row['state'],'disposition':row.get('disposition'),'resolved_state':resolved.get('state'),'effect_count':before.get('cmd_local',0),'observed_ids':bool(row.get('observed_record_ids')),'evidence':normalized_evidence(row.get('evidence'))}
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
            if fault=='crash-after-effect':
                assert 'error' in outcome, outcome
                apple.wait(timeout=5)
                apple = subprocess.Popen([lab, 'simapple', '--socket', str(home/'peer.sock'), '--secret-file', str(secret), '--control', str(home/'control.sock')], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
                time.sleep(.3)
                deadline=time.monotonic()+10
                while not exchange({'t':'Available'})['available'] and time.monotonic()<deadline:
                    time.sleep(.1)
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

    for fault in ['', 'ambiguous-outcome', 'crash-after-effect', 'identical-manual-send', 'ui-interference', 'outcome-before-capture', 'expiry', 'approval', 'owner-rejection']:
        expected=run(go,lab,fault,lifecycle=True)
        actual=run(rust,lab,fault,lifecycle=True)
        assert actual==expected, (fault,expected,actual)
        print(json.dumps({'lifecycle':fault or 'ordinary','go':expected,'rust':actual,'match':True}),flush=True)
