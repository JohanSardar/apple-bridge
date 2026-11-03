import argparse
import http.client
import socket as sockets
import urllib.request
import base64
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from shadow import private_directory


def wait(path, process):
    deadline = time.monotonic() + 30
    while not path.exists():
        if process.poll() is not None or time.monotonic() > deadline:
            raise RuntimeError('synthetic process startup failed')
        time.sleep(.05)


def stop(process):
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    root = private_directory(args.output)
    if list(root.iterdir()):
        raise ValueError('synthetic output must be empty')
    os.environ['APPLE_BRIDGE_SECRET_STORE'] = 'file'
    home = root / 'home'
    socket = root / 'gateway.sock'
    proxy = root / 'proxy.sock'
    log = (root / 'processes.log').open('x')
    gateway_args = ['bin/bridge', 'start', '--role', 'gateway', '--home', str(home), '--socket', str(socket), '--peer-addr', '127.0.0.1:0', '--api-addr', '127.0.0.1:0']
    gateway = subprocess.Popen(gateway_args, stdout=log, stderr=log)
    try:
        wait(socket, gateway)
    finally:
        stop(gateway)
    shutil.copytree(home, root / 'starting')
    gateway = subprocess.Popen(gateway_args, stdout=log, stderr=log)
    tap = helper = None
    try:
        wait(socket, gateway)
        tap = subprocess.Popen([sys.executable, 'lab/shadow/shadow.py', 'record', '--listen', str(proxy), '--upstream', str(socket), '--output', str(root / 'recording')], stdout=log, stderr=log)
        wait(proxy, tap)
        helper = subprocess.Popen(['bin/bridge-lab', 'simapple', '--socket', str(proxy), '--secret-file', str(home / 'apple-secret'), '--control', str(root / 'sim.sock')], stdout=log, stderr=log)
        wait(root / 'sim.sock', helper)
        def control_action(path, value):
            control = http.client.HTTPConnection('sim', timeout=10)
            control.sock = sockets.socket(sockets.AF_UNIX)
            control.sock.connect(str(root / 'sim.sock'))
            control.request('POST', path, body=json.dumps(value), headers={'Content-Type': 'application/json'})
            response = control.getresponse()
            if response.status != 200:
                raise RuntimeError('synthetic control action failed')
            response.read()
            control.close()
        control_action('/seed', {'count': 1})
        deadline = time.monotonic() + 20
        kinds = set()
        while time.monotonic() < deadline:
            recording = root / 'recording/ipc.shadow'
            if recording.exists():
                lines = recording.read_text().splitlines()
                kinds = {json.loads(base64.b64decode(json.loads(line)['raw'])[4:])['t'] for line in lines}
                if 'Capture' in kinds and 'Committed' in kinds:
                    break
            time.sleep(.1)
        if not {'Capture', 'Committed', 'Ready'} <= kinds:
            raise RuntimeError('SimApplePeer capture was not committed')
        token = subprocess.check_output(['bin/bridge', 'token', 'create', '--home', str(home), '--scope', 'admin'], text=True).strip()
        config = json.loads((home / 'runtime.json').read_text())
        origin = 'http://' + config['api_address']
        def operation(name, data):
            request = urllib.request.Request(origin + '/v1/op/' + name, data=json.dumps(data).encode(), headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, timeout=20) as response:
                return json.load(response)
        deadline = time.monotonic() + 10
        records = []
        while not records and time.monotonic() < deadline:
            records = operation('records.query', {'collection': 'reminders.list'})['records']
            time.sleep(.05)
        command = operation('reminders.create', {'command_id': 'cmd_shadow_synthetic', 'op': 'reminders.create', 'args': {'list_id': records[0]['id'], 'title': '[ab-test shadow] synthetic'}, 'wait_ms': 15000})
        if command.get('state') != 'observed':
            raise RuntimeError('synthetic command was not observed')
        control = http.client.HTTPConnection('sim', timeout=5)
        control.sock = sockets.socket(sockets.AF_UNIX)
        control.sock.connect(str(root / 'sim.sock'))
        control.request('POST', '/source-state', body=json.dumps({'source': 'contacts', 'state': 'permission_missing'}), headers={'Content-Type': 'application/json'})
        response = control.getresponse()
        if response.status != 200:
            raise RuntimeError('synthetic SourceState failed')
        response.read()
        control.close()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            kinds = {json.loads(base64.b64decode(json.loads(line)['raw'])[4:])['t'] for line in recording.read_text().splitlines()}
            if 'SourceState' in kinds and 'Prepare' in kinds and 'Dispatch' in kinds:
                break
            time.sleep(.05)
        if not {'SourceState', 'Prepare', 'Dispatch'} <= kinds:
            raise RuntimeError('synthetic command/SourceState recording missing')
    finally:
        for process in (helper, tap, gateway):
            if process:
                stop(process)
        log.close()
    shutil.copytree(home, root / 'identical')
    same = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'diff', '--left', str(home), '--right', str(root / 'identical')], stdout=subprocess.DEVNULL)
    if same.returncode:
        raise RuntimeError('identical synthetic home diff failed')
    import sqlite3
    mutations = [
        ('state.db', 'records', "payload=CAST(payload||x'20' AS BLOB)"),
        ('state.db', 'journal', "payload=CAST(payload||x'20' AS BLOB)"),
        ('state.db', 'aliases', "record_id=record_id||'changed'"),
        ('state.db', 'checkpoints', "checkpoint=checkpoint||'changed'"),
        ('ledger.db', 'commands', "payload=CAST(payload||x'20' AS BLOB)"),
        ('ledger.db', 'dedupe', "request_hash=request_hash||'changed'"),
    ]
    detected = []
    for database, table, update in mutations:
        changed = root / 'changed'
        shutil.copytree(home, changed)
        try:
            with sqlite3.connect(changed / database) as db:
                result = db.execute('UPDATE "' + table + '" SET ' + update + ' WHERE rowid=(SELECT min(rowid) FROM "' + table + '")')
                if result.rowcount != 1:
                    raise RuntimeError('synthetic mutation fixture missing')
            result = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'diff', '--left', str(home), '--right', str(changed)], capture_output=True, text=True)
            report = json.loads(result.stdout)
            mismatches = [row['table'] for row in report['digests'] if not row['equal']]
            if result.returncode != 1 or mismatches != [database + '/' + table]:
                raise RuntimeError('seeded database mismatch was not detected precisely')
            detected.append(database + '/' + table)
        finally:
            shutil.rmtree(changed)
    print(json.dumps({'synthetic_capture_recorded': True, 'identical_home_equal': True, 'changed_gojson_bytes_detected': True, 'seeded_mismatches_detected': detected, 'frame_types': sorted(kinds)}))


if __name__ == '__main__':
    main()
