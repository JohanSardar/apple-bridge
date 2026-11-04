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

from shadow import private_directory, normalized_json, iphone_request


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


def reordered_record_data(raw):
    source = raw.decode() if isinstance(raw, bytes) else raw
    decoder = json.JSONDecoder()
    index = 1
    while True:
        while source[index].isspace() or source[index] == ',':
            index += 1
        key, index = decoder.raw_decode(source, index)
        while source[index].isspace() or source[index] == ':':
            index += 1
        start = index
        value, index = decoder.raw_decode(source, index)
        if key == 'data':
            if not isinstance(value, dict) or len(value) < 2:
                raise RuntimeError('data ordering fixture needs an object')
            changed = json.dumps(dict(reversed(list(value.items()))), separators=(',', ':'))
            result = source[:start] + changed + source[index:]
            if result == source or json.loads(result) != json.loads(source):
                raise RuntimeError('data ordering fault changed semantic values')
            return result.encode() if isinstance(raw, bytes) else result


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    parser.add_argument('--go', default='bin/bridge')
    parser.add_argument('--rust', default='bin/bridge')
    parser.add_argument('--go-go-runs', type=int, default=8)
    parser.add_argument('--lab', default='bin/bridge-lab')
    parser.add_argument('--bridge', default='bin/bridge')
    parser.add_argument('--simapple', default='')
    parser.add_argument('--tools', default='')
    args = parser.parse_args()
    simapple = [args.simapple] if args.simapple else [args.lab, 'simapple']
    tools = args.tools or args.lab
    root = private_directory(args.output)
    if list(root.iterdir()):
        raise ValueError('synthetic output must be empty')
    os.environ['APPLE_BRIDGE_SECRET_STORE'] = 'file'
    home = root / 'home'
    socket = root / 'gateway.sock'
    proxy = root / 'proxy.sock'
    log = (root / 'processes.log').open('x')
    gateway_args = [args.bridge, 'start', '--role', 'gateway', '--home', str(home), '--socket', str(socket), '--peer-addr', '127.0.0.1:0', '--api-addr', '127.0.0.1:0']
    feed_peer = root / 'feed-peer'
    feed_log = root / 'feed.log'
    gateway = subprocess.Popen(gateway_args, stdout=log, stderr=log)
    primer = None
    try:
        wait(socket, gateway)
        wait(home / 'runtime.json', gateway)
        peer_address = json.loads((home / 'runtime.json').read_text())['peer_address']
        code = subprocess.check_output([args.bridge, 'pair', '--home', str(home), '--scope', 'admin', '--peer-addr', peer_address], text=True).splitlines()[0].strip()
        subprocess.run([args.bridge, 'join', '--home', str(feed_peer), code], check=True, stdout=subprocess.DEVNULL, stderr=log)
        token = subprocess.check_output([args.bridge, 'token', 'create', '--home', str(home), '--scope', 'admin'], text=True).strip()
        primer = subprocess.Popen([*simapple, '--socket', str(socket), '--secret-file', str(home / 'apple-secret'), '--control', str(root / 'prime.sock')], stdout=log, stderr=log)
        wait(root / 'prime.sock', primer)
        client = http.client.HTTPConnection('sim', timeout=10)
        client.sock = sockets.socket(sockets.AF_UNIX)
        client.sock.connect(str(root / 'prime.sock'))
        client.request('POST', '/seed', body='{"count":1}', headers={'Content-Type': 'application/json'})
        response = client.getresponse()
        if response.status != 200:
            raise RuntimeError('priming capture failed')
        response.read()
        client.close()
        deadline = time.monotonic() + 15
        import sqlite3
        while time.monotonic() < deadline:
            with sqlite3.connect(home / 'state.db') as db:
                if db.execute("SELECT count(*) FROM records WHERE collection='reminders.list'").fetchone()[0]:
                    break
            time.sleep(.05)
        else:
            raise RuntimeError('priming capture not committed')
    finally:
        if primer:
            stop(primer)
        stop(gateway)
    (root / 'iphone-agent.json').write_text(subprocess.check_output([args.bridge, 'agent', 'create', '--home', str(home), '--name', 'shadow-approver', '--level', 'approve', '--scope', 'iphone:write'], text=True))
    subprocess.run([tools, 'iphone-device', '--home', str(home), '--output', str(root / 'iphone-device.json')], check=True, stdout=log)
    shutil.copytree(home, root / 'starting')
    (home / 'runtime.json').unlink(missing_ok=True)
    observer = root / 'iphone-observer.sock'
    gateway = subprocess.Popen(gateway_args, stdout=log, stderr=log, env=dict(os.environ, APPLE_BRIDGE_LAB_FEED_LOG=str(feed_log), APPLE_BRIDGE_LAB_IPHONE_PLAINTEXT='1', APPLE_BRIDGE_LAB_IPHONE_OBSERVER=str(observer)))
    tap = helper = None
    try:
        wait(socket, gateway)
        wait(home / 'runtime.json', gateway)
        with sockets.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            admission_port = reservation.getsockname()[1]
        tap = subprocess.Popen([sys.executable, 'lab/shadow/shadow.py', 'record', '--listen', str(proxy), '--upstream', str(socket), '--output', str(root / 'recording'), '--iphone-observer', str(observer), '--admission-listen', '127.0.0.1:' + str(admission_port), '--admission-upstream', json.loads((home / 'runtime.json').read_text())['api_address'], '--delegated-log', str(feed_log)], stdout=log, stderr=log)
        wait(proxy, tap)
        helper = subprocess.Popen([*simapple, '--socket', str(proxy), '--secret-file', str(home / 'apple-secret'), '--control', str(root / 'sim.sock')], stdout=log, stderr=log)
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
        control_action('/flush', {})
        deadline = time.monotonic() + 20
        kinds = set()
        while time.monotonic() < deadline:
            recording = root / 'recording/ipc.shadow'
            if recording.exists():
                lines = recording.read_text().splitlines()
                kinds = {json.loads(base64.b64decode(json.loads(line)['raw'])[4:])['t'] for line in lines if json.loads(line)['direction'] not in ('admission', 'response')}
                if 'Capture' in kinds and 'Committed' in kinds:
                    break
            time.sleep(.1)
        if not {'Capture', 'Committed', 'Ready'} <= kinds:
            raise RuntimeError('SimApplePeer capture was not committed')
        config = json.loads((home / 'runtime.json').read_text())
        origin = 'http://127.0.0.1:' + str(admission_port)
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
        generated = operation('reminders.create', {'op': 'reminders.create', 'args': {'list_id': records[0]['id'], 'title': '[ab-test shadow] generated id'}, 'wait_ms': 15000})
        if generated.get('state') != 'observed' or not generated.get('command', {}).get('command_id', '').startswith('cmd_'):
            raise RuntimeError('gateway-generated command id was not observed')
        def http_call(method, path, body=None, extra=None):
            headers = {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'}
            headers.update(extra or {})
            request = urllib.request.Request(origin + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
            with urllib.request.urlopen(request, timeout=20) as response:
                return json.load(response)
        mcp = http_call('POST', '/mcp', {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'records_query', 'arguments': {'collection': 'reminders.list'}}})
        if mcp.get('result', {}).get('isError'):
            raise RuntimeError('synthetic MCP query failed')
        mcp_command = http_call('POST', '/mcp', {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': 'reminders_create', 'arguments': {'command_id': 'cmd_shadow_mcp', 'op': 'reminders.create', 'args': {'list_id': records[0]['id'], 'title': '[ab-test shadow] via mcp'}, 'wait_ms': 15000}}})
        if mcp_command.get('result', {}).get('structuredContent', {}).get('state') != 'observed':
            raise RuntimeError('synthetic MCP command was not observed')
        feed_address = config['peer_address']
        def feed_call(request):
            request['delegation'] = {'local_principal': 'token:shadow-feed', 'local_grants': ['reminders:write']}
            result = subprocess.run([tools, 'shadow-feed', '--peer-home', str(feed_peer), '--address', feed_address], input=json.dumps(request), capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError('synthetic feed request failed: ' + result.stderr.strip())
            return json.loads(result.stdout)['outcome']
        feed_command = feed_call({'method': 'submit', 'command': {'command_id': 'cmd_shadow_feed', 'op': 'reminders.create', 'args': {'list_id': records[0]['id'], 'title': '[ab-test shadow] via feed'}, 'wait_ms': 15000}})
        if feed_command.get('status') != 200 or not feed_command.get('state'):
            raise RuntimeError('synthetic feed command was not admitted')
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            with sqlite3.connect((home / 'ledger.db').as_uri() + '?mode=ro', uri=True) as db:
                row = db.execute("SELECT json_extract(payload,'$.state') FROM commands WHERE id='cmd_shadow_feed'").fetchone()
            if row and row[0] == 'observed':
                break
            time.sleep(.05)
        else:
            raise RuntimeError('synthetic feed command was not observed')
        if feed_call({'method': 'status', 'query': {'command_id': 'cmd_shadow_feed'}}).get('state') != 'observed':
            raise RuntimeError('synthetic feed status was not observed')
        if feed_call({'method': 'list', 'query': {'limit': 10}}).get('status') != 200:
            raise RuntimeError('synthetic feed list failed')
        http_call('GET', '/v1/records/reminders.list/' + records[0]['id'])
        http_call('POST', '/v1/op/records.query', {'collection': 'reminders.list'}, {'X-Ingress': 'cli'})
        device = json.loads((root / 'iphone-device.json').read_text())
        agent = json.loads((root / 'iphone-agent.json').read_text())
        def phone_call(path, payload):
            connection = http.client.HTTPConnection(config['api_address'], timeout=20)
            try:
                connection.request('POST', path, iphone_request(device, path, int(time.time()), json.dumps(payload).encode()), {'Content-Type': 'application/json'})
                response = connection.getresponse()
                response.read()
                return response.status
            finally:
                connection.close()
        pending = urllib.request.Request(origin + '/v1/op/iphone.ping', data=json.dumps({'command_id': 'cmd_shadow_phone', 'op': 'iphone.ping', 'args': {'device_id': device['device_id']}}).encode(), headers={'Authorization': 'Bearer ' + agent['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(pending, timeout=20) as response:
            held = json.load(response)
        if held.get('state') != 'awaiting_approval':
            raise RuntimeError('synthetic iPhone command was not held for approval')
        request_hash = held['request_hash']
        if phone_call('/v1/iphone/approval', {'command_id': 'cmd_shadow_phone', 'request_hash': request_hash, 'approve': True}) != 200:
            raise RuntimeError('synthetic iPhone approval rejected')
        if phone_call('/v1/iphone/pending', {}) != 200:
            raise RuntimeError('synthetic iPhone pending rejected')
        if phone_call('/v1/iphone/result', {'command_id': 'cmd_shadow_phone', 'request_hash': request_hash, 'state': 'observed', 'data': None}) != 200:
            raise RuntimeError('synthetic iPhone result rejected')
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
            kinds = {json.loads(base64.b64decode(json.loads(line)['raw'])[4:])['t'] for line in recording.read_text().splitlines() if json.loads(line)['direction'] not in ('admission', 'response')}
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
        ('state.db', 'records', 'version=version+1'),
        ('state.db', 'records', None),
        ('state.db', 'records', "payload=CAST(json_set(payload,'$.data.observed_at','2099-01-01T00:00:00Z') AS BLOB)"),
        ('state.db', 'records', "payload=CAST(json_set(payload,'$.data.__shadow_fault',1) AS BLOB)"),
        ('state.db', 'journal', 'seq=seq+1000000'),
        ('state.db', 'journal', "payload=CAST(json_set(payload,'$.op','delete') AS BLOB)"),
        ('ledger.db', 'commands', "payload=CAST(json_set(payload,'$.disposition','indeterminate') AS BLOB)"),
        ('ledger.db', 'commands', "payload=CAST(json_set(payload,'$.expires_at','2099-01-01T00:00:00Z') AS BLOB)"),
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
                if update is None:
                    rowid, payload = db.execute("SELECT rowid,payload FROM records WHERE collection='reminders.list' LIMIT 1").fetchone()
                    result = db.execute('UPDATE records SET payload=? WHERE rowid=?', (reordered_record_data(payload), rowid))
                else:
                    result = db.execute('UPDATE "' + table + '" SET ' + update + ' WHERE rowid=(SELECT min(rowid) FROM "' + table + '")')
                if result.rowcount != 1:
                    raise RuntimeError('synthetic mutation fixture missing')
            result = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'diff', '--left', str(home), '--right', str(changed)], capture_output=True, text=True)
            report = json.loads(result.stdout)
            mismatches = [row['table'] for row in report['digests'] if not row['equal']]
            if result.returncode != 1 or mismatches != ([database + '/' + table] if table != 'aliases' else ['state.db/records', 'state.db/journal']):
                raise RuntimeError('seeded database mismatch was not detected precisely')
            detected.append(database + '/' + table)
        finally:
            shutil.rmtree(changed)
    aligned = root / 'aligned'
    shutil.copytree(home, aligned)
    with sqlite3.connect(aligned / 'state.db') as db:
        identity = db.execute("SELECT id FROM records WHERE collection='system.source' LIMIT 1").fetchone()[0]
        replacement = identity + '_synthetic_renamed'
        db.execute('UPDATE aliases SET record_id=? WHERE record_id=?', (replacement, identity))
        db.execute('UPDATE records SET id=? WHERE id=?', (replacement, identity))
        db.execute("UPDATE records SET observed_at='2001-01-01T00:00:00Z'")
        db.execute('UPDATE journal SET committed_at=1')
        for table, clocks in [('records', {('observed_at',): '2001-01-01T00:00:00Z'}), ('journal', {('record', 'observed_at'): '2001-01-01T00:00:00Z'})]:
            rows = db.execute('SELECT rowid,payload FROM ' + table).fetchall()
            for rowid, payload in rows:
                db.execute('UPDATE ' + table + ' SET payload=? WHERE rowid=?', (normalized_json(payload, {identity: replacement}, clocks), rowid))
    result = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'diff', '--left', str(home), '--right', str(aligned)], capture_output=True, text=True)
    if result.returncode or not json.loads(result.stdout)['equal']:
        raise RuntimeError('alias identity and envelope clock equivalence failed')
    manifest = root / 'decisions.json'
    subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'decisions', '--recording', str(recording), '--home', str(home), '--starting-home', str(root / 'starting'), '--output', str(manifest)], check=True, stdout=subprocess.DEVNULL)
    for run in range(args.go_go_runs):
        result = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'replay', '--recording', str(recording), '--starting-home', str(root / 'starting'), '--output', str(root / ('replay-go-go-' + str(run))), '--go', args.go, '--rust', args.go, '--decisions', str(manifest), '--iphone-device', str(root / 'iphone-device.json'), '--exact', '--feed-peer-home', str(feed_peer), '--lab-binary', tools], capture_output=True, text=True)
        report = json.loads(result.stdout) if result.stdout else {'digests': []}
        if result.returncode or not report['digests'] or any(not row['equal'] for row in report['digests']):
            raise RuntimeError('synthetic Go-Go exact replay failed in run ' + str(run) + ': ' + result.stderr.strip())
        shutil.rmtree(root / ('replay-go-go-' + str(run)))
    result = subprocess.run([sys.executable, 'lab/shadow/shadow.py', 'replay', '--recording', str(recording), '--starting-home', str(root / 'starting'), '--output', str(root / 'replay-exact'), '--go', args.go, '--rust', args.rust, '--decisions', str(manifest), '--iphone-device', str(root / 'iphone-device.json'), '--exact', '--feed-peer-home', str(feed_peer), '--lab-binary', tools], capture_output=True, text=True)
    report = json.loads(result.stdout) if result.stdout else {'digests': []}
    if result.returncode or any(not row['equal'] for row in report['digests']):
        raise RuntimeError('synthetic exact decision replay failed: ' + result.stderr.strip())
    with sqlite3.connect(home / 'ledger.db') as db:
        phone_state = db.execute("SELECT state FROM commands WHERE id='cmd_shadow_phone'").fetchone()
    if not phone_state or phone_state[0] != 'observed':
        raise RuntimeError('synthetic iPhone command did not complete')
    ingress = sorted({json.loads(line)['ingress'] for line in recording.read_text().splitlines() if json.loads(line).get('ingress')})
    feed_rows = sum(1 for line in recording.read_text().splitlines() if json.loads(line).get('ingress') == 'feed')
    if feed_rows != 3:
        raise RuntimeError('delegated feed observer recorded ' + str(feed_rows) + ' of 3 commands')
    print(json.dumps({'delegated_feed_commands_recorded': feed_rows, 'exact_decision_replay_equal': True, 'iphone_ingress_rows': sum(1 for line in recording.read_text().splitlines() if json.loads(line).get('ingress') == 'iphone' and json.loads(line)['direction'] == 'admission'), 'ingress_kinds': ingress, 'alias_clock_equivalence': True, 'go_go_exact_replay_runs_equal': args.go_go_runs, 'rust_diff_tables': len(report['digests']), 'synthetic_capture_recorded': True, 'identical_home_equal': True, 'changed_gojson_bytes_detected': True, 'record_data_key_order_detected': True, 'record_data_clock_detected': True, 'seeded_mismatches_detected': detected, 'frame_types': sorted(kinds)}))


if __name__ == '__main__':
    main()
