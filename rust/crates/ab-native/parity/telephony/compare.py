import json
import pathlib
import shlex
import subprocess
import sys
import tempfile

oracle,binary=sys.argv[1:]
script=pathlib.Path(__file__).with_name('generate.py').resolve()
remote=subprocess.check_output(['ssh','build','mktemp -d ~/w/telephony-fixture-XXXXXX'],text=True).strip()
try:
    with tempfile.TemporaryDirectory(prefix='ab-telephony-fixture-') as tmp:
        total=0
        for kind in ['calls','voicemail']:
            root=pathlib.Path(tmp)/kind;remote_root=remote+'/'+kind
            def mutate(operation):
                subprocess.run([sys.executable,str(script),str(root),kind,operation],check=True)
                subprocess.run(['ssh','build',shlex.join(['python3','-',remote_root,kind,operation])],input=script.read_text(),text=True,check=True)
            mutate('initial')
            keeper_code="import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);d.execute('PRAGMA journal_mode=WAL');d.execute('PRAGMA wal_autocheckpoint=0');d.execute('UPDATE Z_METADATA SET Z_UUID=Z_UUID');d.commit();print('ready',flush=True);sys.stdin.read();d.close()"
            keepers=[subprocess.Popen([sys.executable,'-u','-c',keeper_code,str(root/'store.db')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join(['python3','-u','-c',keeper_code,remote_root+'/store.db'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
            for p in keepers: assert p.stdout.readline().strip()=='ready'
            processes=[subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,kind,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join([binary,kind,remote_root,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
            generations=[{},{}];raw=[];captures=[]
            def normalized(frame,index):
                if 'generation' not in frame:return frame
                actual=frame['generation'];known=generations[index];name=known.setdefault(actual,'generation-'+str(len(known)))
                value=json.loads(json.dumps(frame));value['generation']=name
                for row in value['observations']:
                    if 'scoped_alias' in row:row['scoped_alias']=row['scoped_alias'].replace(actual,name)
                return value
            try:
                stages=[('initial',100000),('unchanged',100000),('limit',1),('update',100000),('remove',100000),('duplicate',100000),('repair-duplicate',100000),('negative',100000),('repair-negative',100000),('uuid',100000),('schema',100000),('repair-schema',100000),('clear',100000),('replace',100000)]
                for index,(operation,limit) in enumerate(stages):
                    if operation not in ['initial','unchanged','limit']:mutate(operation)
                    context=json.loads((root/'context.json').read_text());context['checkpoint']=f'synthetic-checkpoint-{index}';context['scan_id']=f'synthetic-scan-{index}'
                    data=json.dumps(context);(root/'context.json').write_text(data)
                    subprocess.run(['ssh','build','cat > '+shlex.quote(remote_root+'/context.json')],input=data,text=True,check=True)
                    frames=[]
                    for i,p in enumerate(processes):
                        p.stdin.write(str(limit)+'\n');p.stdin.flush();line=p.stdout.readline()
                        if not line:raise RuntimeError('reader exited')
                        frame=json.loads(line);raw.append(line);frames.append(normalized(frame,i))
                    assert frames[0]==frames[1],f'{kind}/{operation}: Swift {frames[0]}\nRust {frames[1]}'
                    if operation in ['limit','duplicate','negative','schema']:assert frames[0]=={'kind':'failed_scan'}
                    if operation=='unchanged':assert frames[0]['observations']==[]
                    if operation=='update':
                        assert (root/'store.db-wal').stat().st_size>32
                        assert subprocess.check_output(['ssh','build','test $(stat -c %s '+shlex.quote(remote_root+'/store.db-wal')+') -gt 32 && echo retained'],text=True).strip()=='retained'
                    captures.append(frames[0])
                hashes=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines()
                assert all(hashes[i]==hashes[i+1] for i in range(0,len(hashes),2))
                assert len(generations[0])==len(generations[1])==4
                assert any(r['kind']=='absent' for f in captures for r in f.get('observations',[]))
                total+=len(stages)
                print(f'{kind}: {len(stages)} identical stages, identical Go Data bytes; four matching generation epochs')
            finally:
                for p in processes:p.stdin.close()
                for p in processes:assert p.wait(timeout=10)==0
                for p in keepers:p.stdin.close()
                for p in keepers:assert p.wait(timeout=10)==0
        print(f'Telephony: {total} matching synthetic stages; incomplete/invalid scans emitted no observations')
finally:
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
