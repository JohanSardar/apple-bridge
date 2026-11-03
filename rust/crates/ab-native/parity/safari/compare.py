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

local_mode = "--local" in sys.argv
arguments = [argument for argument in sys.argv[1:] if argument != "--local"]
oracle, binary = arguments
data_bytes = os.environ.get("AB_PARITY_DATA_BYTES")
if local_mode and not data_bytes:
    raise SystemExit("--local requires AB_PARITY_DATA_BYTES")
script = pathlib.Path(__file__).with_name('generate.py').resolve()
remote = None if local_mode else subprocess.check_output(['ssh','build','mktemp -d ~/w/safari-fixture-XXXXXX'],text=True).strip()
def generate_rust(operation=None):
    arguments = [] if operation is None else [operation]
    if local_mode:
        subprocess.run([sys.executable,str(script),remote,*arguments],check=True)
    else:
        subprocess.run(['ssh','build',shlex.join(['python3','-',remote,*arguments])],input=script.read_text(),text=True,check=True)

def swift_command(root):
    return [oracle,str(root)] if local_mode else [str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)]

def rust_command():
    command = [binary,'safari',remote,'--interactive','--frames']
    return command if local_mode else ['ssh','build',shlex.join(command)]

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
        if local_mode: remote=str(pathlib.Path(tmp)/'rust-fixture')
        subprocess.run([sys.executable,str(script),str(root)],check=True)
        generate_rust()
        swift=subprocess.Popen(swift_command(root),stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        rust=subprocess.Popen(rust_command(),stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
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
            generate_rust(operation)
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
        generate_rust('bookmarks-empty')
        swift=subprocess.Popen(swift_command(root),stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        rust=subprocess.Popen(rust_command(),stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        processes.extend([swift,rust]); outputs=[lines(p) for p in processes]
        _,ready=take(outputs[0]);swift_pid=ready['pid'];last_diagnostic[0]=None
        command('scan');equal(3)
        print('identity-cache restart: identical',flush=True)
        digests=subprocess.check_output(([data_bytes] if local_mode else ['ssh','build','~/targets/phase6-native/fixture-data-bytes']),input=''.join(raw),text=True).splitlines()
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
    if not local_mode:
        subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
