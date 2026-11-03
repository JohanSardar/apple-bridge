import json
import os
import pathlib
import queue
import shlex
import signal
import subprocess
import sys
import tempfile
import threading

oracle, binary = sys.argv[1:]
script = pathlib.Path(__file__).with_name('generate.py').resolve()
remote = subprocess.check_output(['ssh','build','mktemp -d ~/w/safari-fixture-XXXXXX'],text=True).strip()
processes=[]
swift_pid=None
paused=False

def lines(process):
    output=queue.Queue()
    def read():
        for line in process.stdout: output.put(line)
        output.put(None)
    threading.Thread(target=read,daemon=True).start()
    return output

def take(output):
    line=output.get(timeout=15)
    if line is None: raise RuntimeError('reader exited')
    return line,json.loads(line)

try:
    with tempfile.TemporaryDirectory(prefix='ab-safari-fixture-') as tmp:
        root=pathlib.Path(tmp)/'fixture'
        subprocess.run([sys.executable,str(script),str(root)],check=True)
        subprocess.run(['ssh','build',shlex.join(['python3','-',remote])],input=script.read_text(),text=True,check=True)
        swift=subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        rust=subprocess.Popen(['ssh','build',shlex.join([binary,'safari',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes=[swift,rust]; outputs=[lines(p) for p in processes]
        _,ready=take(outputs[0]);swift_pid=ready['pid']
        raw=[];captures=[];last_diagnostic=[None]
        def command(kind):
            rust.stdin.write(kind+'\n');rust.stdin.flush()
        def equal(count):
            for _ in range(count):
                right,b=take(outputs[1])
                left,a=take(outputs[0])
                while a == last_diagnostic[0]:
                    left,a=take(outputs[0])
                last_diagnostic[0] = a if a.get('kind') == 'source_state' else None
                if a!=b: raise AssertionError(f'Swift {a}\nRust {b}')
                raw.extend([left,right]);captures.append(a)
        command('scan');equal(3)
        command('scan');assert take(outputs[1])[1]=={'kind':'idle'}
        try: extra=outputs[0].get(timeout=0.4);raise AssertionError('unchanged input emitted '+str(extra))
        except queue.Empty: pass
        stages=[('history-title','history',1),('bookmarks-empty','bookmarks',1),('bookmarks-xml','bookmarks',1),('cloud-clear','cloud',1),('history-clear','history',1),('history-add','history',1),('history-replace','history',2),('history-broken','history',1),('history-repair','history',1),('bookmarks-broken','bookmarks',1),('bookmarks-xml','bookmarks',0),('history-add','history',1),('expiry','history',1)]
        for operation,kind,count in stages:
            os.kill(swift_pid,signal.SIGSTOP);paused=True
            subprocess.run([sys.executable,str(script),str(root),operation],check=True)
            subprocess.run(['ssh','build',shlex.join(['python3','-',remote,operation])],input=script.read_text(),text=True,check=True)
            os.kill(swift_pid,signal.SIGCONT);paused=False
            command(kind)
            if count: equal(count)
            else:
                assert take(outputs[1])[1]=={'kind':'idle'}
            print(operation+': identical',flush=True)
        for p in processes: p.stdin.close()
        for p in processes: assert p.wait(timeout=10) == 0
        processes.clear()
        subprocess.run([sys.executable,str(script),str(root),'bookmarks-empty'],check=True)
        subprocess.run(['ssh','build',shlex.join(['python3','-',remote,'bookmarks-empty'])],input=script.read_text(),text=True,check=True)
        swift=subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        rust=subprocess.Popen(['ssh','build',shlex.join([binary,'safari',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes.extend([swift,rust]); outputs=[lines(p) for p in processes]
        _,ready=take(outputs[0]);swift_pid=ready['pid'];last_diagnostic[0]=None
        command('scan');equal(3)
        print('identity-cache restart: identical',flush=True)
        digests=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines()
        assert all(digests[i]==digests[i+1] for i in range(0,len(digests),2))
        assert any(row.get('kind')=='absent' for frame in captures for row in frame.get('observations',[]))
        print(f'Safari: {len(captures)} identical frames, Go Data byte hashes identical; unchanged input suppressed; malformed input produced no absence')
finally:
    if paused: os.kill(swift_pid,signal.SIGCONT)
    for p in processes:
        p.stdin.close()
    for p in processes:
        try: p.wait(timeout=10)
        except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
