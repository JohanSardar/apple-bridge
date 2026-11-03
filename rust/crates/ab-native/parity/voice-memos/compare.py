import base64
import json
import pathlib
import shlex
import subprocess
import sys
import tempfile

oracle,binary=sys.argv[1:]
script=pathlib.Path(__file__).with_name('generate.py').resolve()
remote=subprocess.check_output(['ssh','build','mktemp -d ~/w/voice-memo-fixture-XXXXXX'],text=True).strip()
processes=[];keepers=[]
try:
    with tempfile.TemporaryDirectory(prefix='ab-voice-memo-fixture-') as tmp:
        root=pathlib.Path(tmp)/'fixture'
        def mutate(operation):
            subprocess.run([sys.executable,str(script),str(root),operation],check=True)
            subprocess.run(['ssh','build',shlex.join(['python3','-',remote,operation])],input=script.read_text(),text=True,check=True)
        mutate('initial')
        code="import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);d.execute('PRAGMA journal_mode=WAL');d.execute('PRAGMA wal_autocheckpoint=0');d.execute('UPDATE Z_METADATA SET Z_UUID=Z_UUID');d.commit();print('ready',flush=True);sys.stdin.read();d.close()"
        keepers=[subprocess.Popen([sys.executable,'-u','-c',code,str(root/'store.db')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join(['python3','-u','-c',code,remote+'/store.db'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        for p in keepers:assert p.stdout.readline().strip()=='ready'
        processes=[subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join([binary,'voice-memos',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        generations=[{},{}];raw=[];captures=[]
        def normal(frame,index):
            if 'generation' not in frame:return frame
            value=json.loads(json.dumps(frame));actual=value['generation'];known=generations[index];name=known.setdefault(actual,'generation-'+str(len(known)));value['generation']=name
            for row in value['observations']:
                if 'scoped_alias' in row:row['scoped_alias']=row['scoped_alias'].replace(actual,name)
            return value
        def exchange(command):
            frames=[]
            for i,p in enumerate(processes):
                p.stdin.write(command+'\n');p.stdin.flush();line=p.stdout.readline()
                if not line:raise RuntimeError('reader exited')
                raw.append(line);frames.append(normal(json.loads(line),i))
            assert frames[0]==frames[1],f'{command}: Swift {frames[0]}\nRust {frames[1]}'
            captures.append(frames[0]);return frames[0]
        staged=None;original=None
        stages=['initial','limit','asset-schema','audio','unchanged','transcript','mutate-audio','remove-audio','escape','symlink','restore','negative','repair-negative','invalid-text','repair-text','uuid','delete','clear']
        for index,operation in enumerate(stages):
            if operation not in ['initial','limit','unchanged']:mutate(operation)
            context=json.loads((root/'context.json').read_text());context['checkpoint']=f'synthetic-checkpoint-{index}';context['scan_id']=f'synthetic-scan-{index}';data=json.dumps(context)
            (root/'context.json').write_text(data);subprocess.run(['ssh','build','cat > '+shlex.quote(remote+'/context.json')],input=data,text=True,check=True)
            if operation=='mutate-audio':
                frame=exchange(f'blob {staged} 0 131072');assert base64.b64decode(frame['bytes'])==original and frame['eof']
                frame=exchange(f'blob {staged} 8 9');assert base64.b64decode(frame['bytes'])==original[8:17] and not frame['eof']
                frame=exchange(f'blob {staged} {len(original)} 1');assert frame['bytes']=='' and frame['eof']
                for offset,length in [(-1,1),(0,0),(0,131073),(len(original)+1,1)]:assert exchange(f'blob {staged} {offset} {length}')=={'kind':'failed_blob'}
                assert exchange('blob voice-memo-blob:unknown 0 1')=={'kind':'failed_blob'}
            frame=exchange('0' if operation=='limit' else 'scan')
            if operation in ['limit','negative','invalid-text']:assert frame=={'kind':'failed_scan'}
            if operation=='unchanged':assert frame['observations']==[]
            if operation=='transcript':
                staged=next(iter(frame['blob_handles'].values()));original=(root/'fixture.m4a').read_bytes()
                row=next(r for r in frame['observations'] if r['native_key']=='fixture');assert row['data']['transcript']==[dict(text='Synthetic',start=0,end=1.5)]
            print(operation+': identical',flush=True)
        hashes=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines();assert all(hashes[i]==hashes[i+1] for i in range(0,len(hashes),2))
        assert len(generations[0])==len(generations[1])==3
        assert any(r['kind']=='absent' for f in captures for r in f.get('observations',[]))
        print(f'Voice Memos: {len(captures)} identical synthetic responses and Go Data hashes, including immutable blob/range checks')
finally:
    for p in processes:p.stdin.close()
    for p in processes:assert p.wait(timeout=10)==0
    for p in keepers:p.stdin.close()
    for p in keepers:assert p.wait(timeout=10)==0
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
