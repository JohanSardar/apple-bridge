import json
import base64
import hashlib
import pathlib
import queue
import shlex
import subprocess
import sys
import tempfile
import threading

oracle,binary=sys.argv[1:]
repo=pathlib.Path(__file__).resolve().parents[5]
script=pathlib.Path(__file__).with_name('generate.py').resolve()
fixtures=repo/'lab/fixtures/notes'
remote_repo='/home/johan/w/phase6-native'
remote=subprocess.check_output(['ssh','build','mktemp -d ~/w/notes-fixture-XXXXXX'],text=True).strip()
remote_parent=remote
remote += '/fixture'
processes=[];keepers=[]
def output(process):
    lines=queue.Queue()
    def read():
        for line in process.stdout:lines.put(line)
        lines.put(None)
    threading.Thread(target=read,daemon=True).start();return lines

def take(lines,timeout=30):
    line=lines.get(timeout=timeout)
    if line is None:raise RuntimeError('reader exited')
    return line,json.loads(line)

def send(process,command):process.stdin.write(command+'\n');process.stdin.flush()
try:
    with tempfile.TemporaryDirectory(prefix='ab-notes-fixture-') as tmp:
        root=pathlib.Path(tmp)/'fixture'
        def mutate(operation):
            subprocess.run([sys.executable,str(script),str(root),operation,str(fixtures)],check=True)
            subprocess.run(['ssh','build',shlex.join(['python3','-',remote,operation,remote_repo+'/lab/fixtures/notes'])],input=script.read_text(),text=True,check=True)
        mutate('initial')
        code="import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);d.execute('PRAGMA journal_mode=WAL');d.execute('PRAGMA wal_autocheckpoint=0');d.execute('UPDATE Z_METADATA SET Z_UUID=Z_UUID');d.commit();print('ready',flush=True);sys.stdin.read();d.close()"
        keepers=[subprocess.Popen([sys.executable,'-u','-c',code,str(root/'NoteStore.sqlite')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join(['python3','-u','-c',code,remote+'/NoteStore.sqlite'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        for p in keepers:assert p.stdout.readline().strip()=='ready'
        swift=subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes=[swift];swift_output=output(swift)
        assert take(swift_output,180)[1]=={'kind':'ready'}
        send(swift,'facts');_,facts=take(swift_output)
        context=json.loads((root/'context.json').read_text());context['facts']=facts['facts']
        def save_context(index,paths=False):
            context['checkpoint']=f'synthetic-checkpoint-{index}';context['scan_id']=f'synthetic-scan-{index}'
            context['asset_paths']=[str(root/'Accounts')] if paths else []
            (root/'context.json').write_text(json.dumps(context))
            remote_context=dict(context,asset_paths=[remote+'/Accounts'] if paths else [])
            subprocess.run(['ssh','build','cat > '+shlex.quote(remote+'/context.json')],input=json.dumps(remote_context),text=True,check=True)
        save_context(0)
        rust=subprocess.Popen(['ssh','build',shlex.join([binary,'notes',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes.append(rust);outputs=[swift_output,output(rust)];raw=[];captures=[]
        def exchange(command):
            send(processes[0],command)
            swift=[]
            while True:
                line,frame=take(outputs[0]);swift.append((line,frame))
                if frame.get('kind') in ['done','failed_scan']:break
            updated=False
            for _,frame in swift:
                for row in frame.get('observations',[]):
                    for item in row.get('data',{}).get('attachments',[]):
                        if 'variant_status' not in item:continue
                        original=item['blob']['digest'];variants={}
                        for name,reference in item.get('derivatives',{}).items():
                            data=bytearray()
                            while True:
                                send(processes[0],f'blob notes-blob:{reference["digest"]} {len(data)} 131072')
                                _,chunk=take(outputs[0]);data.extend(base64.b64decode(chunk['bytes']))
                                if chunk['eof']:break
                            variants[name]=dict(bytes_base64=base64.b64encode(data).decode(),mime=reference['mime'],width=reference.get('width'),height=reference.get('height'))
                        context['facts'].setdefault('derivatives',{})[original]=dict(status=item['variant_status'],variants=variants);updated=True
            if updated:
                (root/'context.json').write_text(json.dumps(context))
                remote_context=dict(context,asset_paths=[remote+'/Accounts'] if context.get('asset_paths') else [])
                subprocess.run(['ssh','build','cat > '+shlex.quote(remote+'/context.json')],input=json.dumps(remote_context),text=True,check=True)
            send(processes[1],command)
            frames=[]
            for left,a in swift:
                right,b=take(outputs[1])
                if a!=b:
                    if a.keys()!=b.keys():raise AssertionError(f'{command}: Swift keys {a.keys()} Rust keys {b.keys()}')
                    for key in a:
                        if a[key]!=b[key]:
                            if key=='observations':
                                for i,(one,two) in enumerate(zip(a[key],b[key])):
                                    if one!=two:
                                        if one.keys()!=two.keys():raise AssertionError(f'observation fields {one.keys()} / {two.keys()}')
                                        for field in one:
                                            if one[field]!=two[field]:raise AssertionError(f'{command} observation {i} {one.get("native_key")} {field}: Swift {str(one[field])[:1200]} Rust {str(two[field])[:1200]}')
                                raise AssertionError(f'observation count Swift {len(a[key])} Rust {len(b[key])}')
                            raise AssertionError(f'{command} {key}: Swift {a[key]} Rust {b[key]}')
                raw.extend([left,right]);frames.append(a);captures.append(a)
            return frames
        def blob(handle,offset,length):
            for process in processes:send(process,f'blob {handle} {offset} {length}')
            left,a=take(outputs[0]);right,b=take(outputs[1]);assert a==b,(a,b)
            raw.extend([left,right]);captures.append(a);return a
        def all_bytes(reference):
            handle='notes-blob:'+reference['digest'];data=bytearray()
            while True:
                chunk=blob(handle,len(data),131072);data.extend(base64.b64decode(chunk['bytes']))
                if chunk['eof']:break
            assert len(data)==reference['size'] and hashlib.sha256(data).hexdigest()==reference['digest']
            return bytes(data)
        original_attachment=None
        stages=['initial','unchanged','bookkeeping','no-revision','repair-no-revision','title','body','body-repair','locked','unlocked','hold','held-title','held-title2','release','hold','body','locked','release','unlocked','remove-media','restore-media','change-media','escape-media','repair-media','orphan-attachment','repair-attachment','image','smart','unsmart','account','cycle-folder','repair-cycle','trash','untrash','rename-key','delete-note','mark-note','missing-revisions','repair-revisions','schema','uuid']
        for index,operation in enumerate(stages):
            if operation not in ['initial','unchanged','hold','release']:mutate(operation)
            paths=operation in ['remove-media','restore-media','change-media','escape-media','repair-media']
            save_context(index,paths)
            frames=exchange(operation if operation in ['hold','release'] else 'scan')
            if operation in ['unchanged','no-revision','repair-no-revision']:
                assert frames[0]['observations']==[] and frames[-1]['changed_notes']==0
            if operation=='missing-revisions':assert frames==[{'kind':'failed_scan'}]
            if operation=='change-media':
                assert all_bytes(original_attachment)=='[ab-test] exact Notes attachment bytes 🧪'.encode()
            if operation=='image':
                attachments=[item for frame in frames for row in frame.get('observations',[]) for item in row.get('data',{}).get('attachments',[])]
                assert any(item.get('derivatives') for item in attachments), 'synthetic PNG produced no platform variants'
            if operation=='initial':
                notes={row['native_key']:row['data'] for f in frames for row in f.get('observations',[]) if row['collection']=='notes.note'}
                assert notes['native-plain']['editable']=='plain' and notes['native-rich']['editable']=='none'
                assert notes['large']['body_status']=='external' and notes['malformed']['content_status']=='undecoded'
                assert 'deleted' not in notes
                body=all_bytes(notes['large']['body_blob']);assert json.loads(body)['plain_text']=='[ab-test] Large\n'+'x'*250000+'\n'
                assert all_bytes(notes['malformed']['raw_body'])==(fixtures/'malformed.gz').read_bytes()
                original_attachment=next(item['blob'] for item in notes['note']['attachments'] if item['id']=='fixture-image')
                original_bytes=all_bytes(original_attachment);assert original_bytes=='[ab-test] exact Notes attachment bytes 🧪'.encode()
                handle='notes-blob:'+original_attachment['digest']
                assert base64.b64decode(blob(handle,3,7)['bytes'])==original_bytes[3:10]
                for offset,length in [(-1,1),(0,0),(0,131073),(len(original_bytes)+1,1)]:assert blob(handle,offset,length)=={'kind':'failed_blob'}
                assert blob('notes-blob:unknown',0,1)=={'kind':'failed_blob'}
            print(operation+': identical',flush=True)
        for p in processes:p.stdin.close()
        for p in processes:assert p.wait(timeout=10)==0
        processes.clear()
        mutate('restart-delete');save_context('restart')
        swift=subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        rust=subprocess.Popen(['ssh','build',shlex.join([binary,'notes',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes.extend([swift,rust]);outputs=[output(p) for p in processes]
        assert take(outputs[0],180)[1]=={'kind':'ready'}
        frames=exchange('scan')
        assert any(row['kind']=='absent' and row['native_key']=='split-title' for frame in frames for row in frame.get('observations',[]))
        print('identity-cache restart: identical',flush=True)
        mutate('empty');save_context('empty');exchange('scan')
        print('complete empty scan: identical',flush=True)
        hashes=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines();assert all(hashes[i]==hashes[i+1] for i in range(0,len(hashes),2))
        print(f'Notes: {len(stages)+2} matching stages, {len(captures)} responses and identical Go Data byte hashes')
finally:
    for p in processes:p.stdin.close()
    for p in processes:
        try:assert p.wait(timeout=10)==0
        except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
    for p in keepers:p.stdin.close()
    for p in keepers:assert p.wait(timeout=10)==0
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote_parent)],check=True)
