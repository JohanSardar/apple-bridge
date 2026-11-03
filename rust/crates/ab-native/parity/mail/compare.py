import json
import pathlib
import shlex
import subprocess
import sys
import tempfile

oracle,binary=sys.argv[1:]
script=pathlib.Path(__file__).with_name('generate.py').resolve()
remote=subprocess.check_output(['ssh','build','mktemp -d ~/w/mail-fixture-XXXXXX'],text=True).strip()
processes=[];keepers=[]
try:
    with tempfile.TemporaryDirectory(prefix='ab-mail-fixture-') as tmp:
        root=pathlib.Path(tmp)/'fixture'
        def mutate(operation):
            subprocess.run([sys.executable,str(script),str(root),operation],check=True)
            subprocess.run(['ssh','build',shlex.join(['python3','-',remote,operation])],input=script.read_text(),text=True,check=True)
        mutate('initial')
        code="import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);d.execute('PRAGMA journal_mode=WAL');d.execute('PRAGMA wal_autocheckpoint=0');d.execute('UPDATE messages SET read=read WHERE ROWID=1');d.commit();print('ready',flush=True);sys.stdin.read();d.close()"
        keepers=[subprocess.Popen([sys.executable,'-u','-c',code,str(root/'store.db')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join(['python3','-u','-c',code,remote+'/store.db'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        for p in keepers:assert p.stdout.readline().strip()=='ready'
        processes=[subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join([binary,'mail',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        raw=[];captures=[];generations=[{},{}]
        def normal(frame,index):
            if 'generation' not in frame:return frame
            actual=frame['generation'];known=generations[index];frame['generation']=known.setdefault(actual,'generation-'+str(len(known)));return frame
        def exchange(command):
            frames=[]
            for index,p in enumerate(processes):
                p.stdin.write(command+'\n');p.stdin.flush();line=p.stdout.readline()
                if not line:raise RuntimeError('reader exited')
                raw.append(line);frames.append(normal(json.loads(line),index))
            if frames[0]!=frames[1]:
                a,b=frames
                if a.keys()!=b.keys():raise AssertionError(f'different frame keys {a.keys()} / {b.keys()}')
                for key in a:
                    if a[key]!=b[key]:
                        if key=='observations':
                            for i,(left,right) in enumerate(zip(a[key],b[key])):
                                if left!=right:raise AssertionError(f'{command} observation {i}: Swift {left} Rust {right}')
                            raise AssertionError(f'observation count Swift {len(a[key])} Rust {len(b[key])}')
                        raise AssertionError(f'{key}: Swift {a[key]} Rust {b[key]}')
            captures.append(frames[0]);return frames[0]
        stages=[('initial',100000),('unchanged',100000),('partial',10),('remove',100000),('heal',100000),('unchanged',100000),('update',100000),('invalid-url',100000),('repair-url',100000),('alias',100000),('missing-address',100000),('missing-mailbox',100000),('repair-mailbox',100000),('invalid-text',100000),('repair-text',100000),('clear',100000),('heal',100000),('unchanged',100000),('replace',100000)]
        for index,(operation,limit) in enumerate(stages):
            if operation=='replace':
                for p in keepers:p.stdin.close()
                for p in keepers:assert p.wait(timeout=10)==0
                keepers.clear()
            if operation not in ['initial','unchanged','partial','heal']:mutate(operation)
            context=json.loads((root/'context.json').read_text());context['checkpoint']=f'synthetic-checkpoint-{index}';context['scan_id']=f'synthetic-scan-{index}';data=json.dumps(context)
            (root/'context.json').write_text(data);subprocess.run(['ssh','build','cat > '+shlex.quote(remote+'/context.json')],input=data,text=True,check=True)
            frame=exchange(str(limit))
            if operation=='partial':
                assert frame['completion_evidence']=='partial'
                assert sum(r['collection']=='mail.message' for r in frame['observations'])==10
                assert not any(r['kind']=='absent' for r in frame['observations'])
            if operation=='remove':assert any(r['kind']=='absent' and r['native_key']=='1000' for r in frame['observations'])
            if operation in ['invalid-url','missing-mailbox','invalid-text']:assert frame=={'kind':'failed_scan'}
            if operation=='unchanged':assert frame=={'kind':'idle'}
            if operation=='replace':assert frame['completion_evidence']=='complete' and not any(r['kind']=='absent' for r in frame['observations'])
            print(operation+': identical',flush=True)
        hashes=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines();assert all(hashes[i]==hashes[i+1] for i in range(0,len(hashes),2))
        assert len(generations[0])==len(generations[1])==2
        print(f'Mail: {len(captures)} identical synthetic frames, ordered observations and Go Data hashes; partial scans preserve absence candidates')
finally:
    for p in processes:p.stdin.close()
    for p in processes:assert p.wait(timeout=10)==0
    for p in keepers:p.stdin.close()
    for p in keepers:assert p.wait(timeout=10)==0
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
