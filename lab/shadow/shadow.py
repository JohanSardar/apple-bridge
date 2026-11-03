import argparse
import concurrent.futures
import http.client
import http.server
import base64
from contextlib import ExitStack
from datetime import datetime
import hashlib
import fcntl
import hmac
import re
import json
import itertools
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import sqlite3
import struct
import subprocess
import sys
import threading
import time


MAX_FRAME = 16 * 1024 * 1024


def private_directory(path):
    path = Path(path).resolve()
    if path.is_relative_to(Path(__file__).resolve().parents[2]):
        raise ValueError('shadow data must be outside the git checkout')
    existing = path
    while not existing.exists():
        existing = existing.parent
    root = subprocess.run(['git', '-C', str(existing), 'rev-parse', '--show-toplevel'], capture_output=True, text=True)
    if root.returncode == 0 and path.is_relative_to(Path(root.stdout.strip()).resolve()):
        raise ValueError('shadow data must be outside the git checkout')
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    if path.stat().st_uid != os.getuid() or path.stat().st_mode & 0o077:
        raise ValueError('shadow directory must be owner-only')
    return path


def receive(conn):
    def exact(size):
        data = bytearray()
        while len(data) < size:
            part = conn.recv(size - len(data))
            if not part:
                raise EOFError()
            data.extend(part)
        return bytes(data)
    head = exact(4)
    size = struct.unpack('!I', head)[0]
    if not 0 < size <= MAX_FRAME:
        raise ValueError('invalid frame length')
    return head + exact(size)


def frame(raw):
    return json.loads(raw[4:])


def send(conn, value):
    raw = json.dumps(value, separators=(',', ':')).encode()
    conn.sendall(struct.pack('!I', len(raw)) + raw)


def record(args):
    directory = private_directory(args.output)
    path = directory / 'ipc.shadow'
    listen = Path(args.listen).resolve()
    private_directory(listen.parent)
    if listen.exists():
        raise ValueError('proxy socket already exists')
    stop = threading.Event()
    workers = []
    peers = set()
    remaining_relays = {}
    lock = threading.Lock()
    errors = []
    sequence = 0
    deadline = time.monotonic() + args.duration if args.duration else None
    session = 0
    started = time.monotonic_ns()
    def append(direction, raw, number, **metadata):
        nonlocal sequence
        with lock:
            sequence += 1
            output.write(json.dumps(dict(sequence=sequence, session=number, ns=time.monotonic_ns() - started, direction=direction, raw=base64.b64encode(raw).decode(), **metadata)) + '\n')
            output.flush()
            return sequence
    with open(path, 'x', opener=lambda p, f: os.open(p, f, 0o600)) as output, socket.socket(socket.AF_UNIX) as server:
        fcntl.flock(output, fcntl.LOCK_EX | fcntl.LOCK_NB)
        server.bind(str(listen))
        os.chmod(listen, 0o600)
        server.listen(1)
        server.settimeout(.2)
        previous = signal.signal(signal.SIGTERM, lambda *_: stop.set())
        interrupt = signal.signal(signal.SIGINT, lambda *_: stop.set())
        def relay(source, target, direction, number):
            try:
                while not stop.is_set():
                    raw = receive(source)
                    try:
                        append(direction, raw, number)
                    except OSError:
                        errors.append('recording disk write failed')
                        stop.set()
                        return
                    target.sendall(raw)
            except EOFError:
                pass
            except OSError:
                if not stop.is_set():
                    errors.append('tap forwarding failed')
                    stop.set()
            except Exception:
                errors.append('recording failed')
                stop.set()
            finally:
                try:
                    target.shutdown(socket.SHUT_WR)
                except OSError:
                    pass
                with lock:
                    remaining_relays[number] -= 1
                    if remaining_relays[number] == 0:
                        del remaining_relays[number]
                        for conn in (source, target):
                            peers.discard(conn)
                            conn.close()
        admission_server = None
        if args.admission_listen:
            host, port = args.admission_listen.rsplit(':', 1)
            if host != '127.0.0.1' or not args.admission_upstream or not args.admission_upstream.startswith('127.0.0.1:'):
                raise ValueError('admission tap requires loopback listener and upstream')
            class Admissions(http.server.BaseHTTPRequestHandler):
                def setup(self):
                    self.request.settimeout(60)
                    super().setup()
                def log_message(self, *_):
                    pass
                def do_POST(self):
                    if self.headers.get('Transfer-Encoding') or not self.path.startswith('/v1/op/'):
                        self.send_error(400)
                        return
                    size = int(self.headers.get('Content-Length', '0'))
                    if not 0 < size <= MAX_FRAME:
                        self.send_error(400)
                        return
                    body = self.rfile.read(size)
                    if len(body) != size:
                        self.send_error(400)
                        return
                    headers = {key: value for key, value in self.headers.items() if key.lower() not in ('host', 'connection')}
                    try:
                        number = session
                        request = append('admission', body, number, path=self.path, headers=headers)
                        conn = http.client.HTTPConnection(args.admission_upstream, timeout=60)
                        try:
                            conn.request('POST', self.path, body, headers)
                            result = conn.getresponse()
                            response = result.read()
                            append('response', response, number, request=request, status=result.status)
                            self.send_response(result.status)
                            for key, value in result.getheaders():
                                if key.lower() not in ('content-length', 'transfer-encoding', 'connection'):
                                    self.send_header(key, value)
                            self.send_header('Content-Length', str(len(response)))
                            self.end_headers()
                            self.wfile.write(response)
                        finally:
                            conn.close()
                    except (OSError, http.client.HTTPException):
                        errors.append('admission tap failed')
                        stop.set()
            admission_server = http.server.ThreadingHTTPServer((host, int(port)), Admissions)
            admission_server.daemon_threads = False
            threading.Thread(target=admission_server.serve_forever, daemon=True).start()
        try:
            while not stop.is_set() and (deadline is None or time.monotonic() < deadline):
                try:
                    helper, _ = server.accept()
                except socket.timeout:
                    continue
                gateway = socket.socket(socket.AF_UNIX)
                gateway.connect(args.upstream)
                session += 1
                with lock:
                    peers.update((helper, gateway))
                    remaining_relays[session] = 2
                for worker in [threading.Thread(target=relay, args=(helper, gateway, 'helper', session)), threading.Thread(target=relay, args=(gateway, helper, 'gateway', session))]:
                    workers.append(worker)
                    worker.start()
                workers = [worker for worker in workers if worker.is_alive()]
        finally:
            stop.set()
            if admission_server:
                admission_server.shutdown()
                admission_server.server_close()
            with lock:
                closing = list(peers)
            for conn in closing:
                try:
                    conn.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                conn.close()
            for worker in workers:
                worker.join()
            signal.signal(signal.SIGTERM, previous)
            signal.signal(signal.SIGINT, interrupt)
            listen.unlink(missing_ok=True)
    if errors:
        raise ValueError(errors[0])
    print(json.dumps({'recorded_frames': sequence, 'sessions': session}))


def connect(path, secret):
    conn = socket.socket(socket.AF_UNIX)
    conn.settimeout(15)
    conn.connect(str(path))
    challenge = frame(receive(conn))
    if challenge.get('t') != 'Challenge':
        raise ValueError('gateway challenge missing')
    nonce = secrets.token_hex(32)
    def proof(role):
        return hmac.new(secret, ('apple-bridge-ipc/1\0' + role + '\0' + challenge['nonce'] + '\0' + nonce).encode(), hashlib.sha256).hexdigest()
    send(conn, {'t': 'Authenticate', 'nonce': nonce, 'proof': proof('helper')})
    authenticated = frame(receive(conn))
    if authenticated.get('t') != 'Authenticated' or not hmac.compare_digest(authenticated.get('proof', ''), proof('gateway')):
        raise ValueError('gateway authentication failed')
    opened = frame(receive(conn))
    if opened.get('t') != 'Open':
        raise ValueError('gateway Open missing')
    return conn


def admit(row, address):
    conn = http.client.HTTPConnection(address, timeout=60)
    try:
        conn.request('POST', row['path'], base64.b64decode(row['raw'], validate=True), row['headers'])
        result = conn.getresponse()
        response = json.loads(result.read())
        return result.status, response
    finally:
        conn.close()


def replay_session(rows, path, secret, api_address, home):
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        conn = connect(path, secret)
        admissions = {}
        source_states = {}
        for row in rows:
            if row['direction'] == 'admission':
                admissions[row['sequence']] = pool.submit(admit, row, api_address)
                continue
            if row['direction'] == 'response':
                status, response = admissions.pop(row['request']).result(timeout=60)
                recorded = json.loads(base64.b64decode(row['raw'], validate=True))
                if status != row['status'] or not isinstance(response, dict) or not isinstance(recorded, dict) or any(response.get(key) != recorded.get(key) for key in ('state', 'disposition', 'error')):
                    raise ValueError('replay admission response mismatch at sequence ' + str(row['sequence']))
                continue
            raw = base64.b64decode(row['raw'], validate=True)
            value = frame(raw)
            kind = value.get('t')
            if kind in ('Challenge', 'Authenticate', 'Authenticated', 'Open', 'Ping', 'Pong'):
                continue
            if row['direction'] == 'helper':
                conn.sendall(raw)
                if kind == 'SourceState':
                    source_states[value['source']] = value['state']
            else:
                deadline = time.monotonic() + 15
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise ValueError('replay gateway response deadline exceeded')
                    conn.settimeout(remaining)
                    try:
                        actual = receive(conn)
                    except TimeoutError as error:
                        for request, admission in admissions.items():
                            if admission.done():
                                status, result = admission.result()
                                state = result.get('state') if isinstance(result, dict) else None
                                problem = result.get('error') if isinstance(result, dict) else None
                                if status >= 400 or problem or state in ('rejected', 'failed', 'expired'):
                                    code = problem.get('code') if isinstance(problem, dict) else None
                                    raise ValueError('replay admission ' + str(request) + ' cannot produce ' + str(kind) + ': HTTP ' + str(status) + ', state=' + str(state) + ', code=' + str(code)) from error
                        raise ValueError('replay response timeout at sequence ' + str(row['sequence'])) from error
                    if frame(actual).get('t') == 'Ping':
                        send(conn, {'t': 'Pong'})
                        continue
                    break
                if frame(actual).get('t') != kind:
                    raise ValueError('replay gateway frame type mismatch at sequence ' + str(row['sequence']) + ': expected ' + str(kind) + ', received ' + str(frame(actual).get('t')))
                if kind in ('Committed', 'Credit') and frame(actual) != value:
                    raise ValueError('replay acknowledgment mismatch at sequence ' + str(row['sequence']))
                clocks = {('expires_at',)} if kind == 'Prepare' else set()
                if kind not in ('Committed', 'Credit') and normalized_json(actual[4:], {}, clocks) != normalized_json(raw[4:], {}, clocks):
                    raise ValueError('replay requires matching command/query traffic at sequence ' + str(row['sequence']))
        for admission in admissions.values():
            admission.result(timeout=60)
        send(conn, {'t': 'Ping'})
        deadline = time.monotonic() + 15
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError('replay completion deadline exceeded')
            conn.settimeout(remaining)
            kind = frame(receive(conn)).get('t')
            if kind == 'Pong':
                break
            if kind != 'Ping':
                raise ValueError('replay ended with unexpected gateway traffic')
            send(conn, {'t': 'Pong'})
        deadline = time.monotonic() + 15
        while source_states:
            with sqlite3.connect((home / 'state.db').as_uri() + '?mode=ro', uri=True) as db:
                actual = {}
                for payload, in db.execute("SELECT payload FROM records WHERE collection='system.source'"):
                    record = json.loads(payload)
                    actual[record['data']['source']] = record['data']['state']
            source_states = {key: value for key, value in source_states.items() if actual.get(key) != value}
            if source_states:
                if time.monotonic() >= deadline:
                    raise ValueError('replay SourceState persistence deadline exceeded')
                time.sleep(.01)
        return conn


def recording_rows(path):
    previous_session = 0
    with open(path) as recording:
        for index, line in enumerate(recording, 1):
            row = json.loads(line)
            raw = base64.b64decode(row['raw'], validate=True)
            if row['sequence'] != index or row['direction'] not in ('helper', 'gateway', 'admission', 'response'):
                raise ValueError('invalid recording at sequence ' + str(index))
            if not isinstance(row['session'], int) or row['session'] < max(1, previous_session):
                raise ValueError('overlapping or missing IPC sessions are not supported')
            previous_session = row['session']
            if row['direction'] == 'response':
                if not isinstance(row['request'], int) or not isinstance(row['status'], int):
                    raise ValueError('invalid response metadata')
                json.loads(raw)
            elif row['direction'] == 'admission':
                if not row['path'].startswith('/v1/op/') or not isinstance(row['headers'], dict):
                    raise ValueError('invalid admission metadata')
                json.loads(raw)
            elif len(raw) < 5 or len(raw) > MAX_FRAME + 4 or struct.unpack('!I', raw[:4])[0] != len(raw) - 4:
                raise ValueError('invalid recording at sequence ' + str(index))
            else:
                frame(raw)
            yield row


def replay(args):
    destination = private_directory(args.output)
    original = Path(args.starting_home).resolve()
    if destination.is_relative_to(original) or original.is_relative_to(destination):
        raise ValueError('starting home and output must be disjoint')
    recording_lock = open(args.recording, 'rb')
    fcntl.flock(recording_lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
    starting_lock = open(original / 'lock', 'rb')
    fcntl.flock(starting_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    pending = set()
    commands = set()
    requests = set()
    captures = 0
    for row in recording_rows(args.recording):
        if row['direction'] == 'admission':
            requests.add(row['sequence'])
            continue
        if row['direction'] == 'response':
            if row['request'] not in requests:
                raise ValueError('unmatched admission response')
            requests.remove(row['request'])
            continue
        value = frame(base64.b64decode(row['raw']))
        if row['direction'] == 'gateway' and value.get('t') == 'Prepare':
            commands.add(value['command_id'])
        if row['direction'] == 'helper' and value.get('t') in ('Outcome', 'Rejected'):
            commands.discard(value['command_id'])
        if row['direction'] == 'helper' and value.get('t') == 'Capture':
            captures += 1
            capture = value.get('capture', value)
            pending.add((row['session'], capture['source'], capture['checkpoint']))
        if row['direction'] == 'gateway' and value.get('t') == 'Committed':
            pending.discard((row['session'], value['source'], value['checkpoint']))
    if pending or requests or commands:
        raise ValueError('recording ends with unacknowledged captures, admissions or commands')
    if not captures:
        raise ValueError('recording has no captures')
    for binary, name in ((args.go_binary, 'go'), (args.rust_binary, 'rust')):
        home = destination / name
        if home.exists():
            raise ValueError('replay home already exists')
        for entry in original.rglob('*'):
            if entry.is_symlink():
                raise ValueError('starting home contains symlinks')
        shutil.copytree(original, home, ignore=shutil.ignore_patterns('*.sock', '*-wal', '*-shm', 'runtime.json', '*.lock', 'lock', 'pid'))
        for database in original.glob('*.db'):
            with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as source, sqlite3.connect(home / database.name) as target:
                source.backup(target)
        path = destination / (name + '.sock')
        env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file')
        with open(destination / (name + '.log'), 'x', opener=lambda p, f: os.open(p, f, 0o600)) as log:
            process = subprocess.Popen([str(Path(binary).resolve()), 'start', '--role', 'gateway', '--home', str(home), '--socket', str(path), '--peer-addr', '127.0.0.1:0', '--api-addr', '127.0.0.1:0'], stdout=log, stderr=log, env=env)
            active_conn = None
            try:
                deadline = time.monotonic() + 30
                while not path.exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise ValueError(name + ' blocked: gateway ipc/1 startup unavailable; private log retained')
                    time.sleep(.05)
                secret = (home / 'apple-secret').read_bytes()
                runtime = home / 'runtime.json'
                while not runtime.exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise ValueError('replay API readiness unavailable')
                    time.sleep(.05)
                address = json.loads(runtime.read_text())['api_address']
                for session, rows in itertools.groupby(recording_rows(args.recording), key=lambda row: row['session']):
                    if active_conn:
                        active_conn.close()
                    active_conn = replay_session(rows, path, secret, address, home)
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                if active_conn:
                    active_conn.close()
    return diff(argparse.Namespace(left=destination / 'go', right=destination / 'rust'))


def duration(value):
    match = re.fullmatch(r'([0-9]+(?:\.[0-9]+)?)(s|m|h)', value)
    if not match or float(match[1]) <= 0:
        raise argparse.ArgumentTypeError('duration must be positive with s, m or h suffix')
    return float(match[1]) * {'s': 1, 'm': 60, 'h': 3600}[match[2]]


def identities(home):
    path = Path(home).resolve() / 'state.db'
    groups = {}
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
        for source, generation, collection, native, scoped, record in db.execute('SELECT source,generation,collection,native_key,scoped_alias,record_id FROM aliases'):
            groups.setdefault(record, []).append((source, generation, collection, native, scoped))
    return {record: '@alias:' + hashlib.sha256(json.dumps(sorted(keys), separators=(',', ':')).encode()).hexdigest() for record, keys in groups.items()}


def normalized_json(raw, mapping, clock_paths):
    source = raw.decode() if isinstance(raw, bytes) else raw
    decoder = json.JSONDecoder()
    edits = []
    def space(index):
        while index < len(source) and source[index].isspace():
            index += 1
        return index
    def walk(index, path):
        index = space(index)
        start = index
        if source[index] == '{':
            index = space(index + 1)
            while source[index] != '}':
                key, end = decoder.raw_decode(source, index)
                index = space(end)
                if source[index] != ':':
                    raise ValueError('invalid JSON object')
                index = space(walk(index + 1, path + (key,)))
                if source[index] == '}':
                    break
                if source[index] != ',':
                    raise ValueError('invalid JSON object separator')
                index = space(index + 1)
            return index + 1
        if source[index] == '[':
            index = space(index + 1)
            while source[index] != ']':
                index = space(walk(index, path + ('*',)))
                if source[index] == ']':
                    break
                if source[index] != ',':
                    raise ValueError('invalid JSON array separator')
                index = space(index + 1)
            return index + 1
        value, end = decoder.raw_decode(source, index)
        if path in clock_paths:
            if not isinstance(value, str):
                raise ValueError('clock must be a string')
            edits.append((start, end, json.dumps(clock_paths[path] if isinstance(clock_paths, dict) else '@clock')))
        elif isinstance(value, str) and value in mapping and 'data' not in path and 'args' not in path and 'evidence' not in path:
            edits.append((start, end, json.dumps(mapping[value])))
        return end
    if space(walk(0, ())) != len(source):
        raise ValueError('trailing JSON data')
    for start, end, value in reversed(edits):
        source = source[:start] + value + source[end:]
    return source.encode() if isinstance(raw, bytes) else source


def timestamp_ns(value):
    match = re.fullmatch(r'(.*T[0-9]{2}:[0-9]{2}:[0-9]{2})(?:\.([0-9]{1,9}))?(Z|[+-][0-9]{2}:[0-9]{2})', value)
    if not match:
        raise ValueError('invalid command clock')
    seconds = int(datetime.fromisoformat(match[1] + match[3].replace('Z', '+00:00')).timestamp())
    return seconds * 1000000000 + int((match[2] or '').ljust(9, '0'))


def database_digest(home, database, table, mapping):
    path = Path(home).resolve() / database
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
        columns = db.execute('PRAGMA table_info("' + table + '")').fetchall()
        if not columns:
            raise ValueError('required comparison table missing: ' + database + '/' + table)
        rows = db.execute('SELECT * FROM "' + table + '"')
        digests = []
        names = [column[1] for column in columns]
        for row in rows:
            digest = hashlib.sha256()
            for name, value in zip(names, row):
                if database == 'state.db' and ((table == 'records' and name == 'observed_at') or (table == 'journal' and name == 'committed_at')):
                    value = '@clock'
                elif name in ('id', 'record_id') and isinstance(value, str):
                    value = mapping.get(value, value)
                elif name == 'payload' and table in ('records', 'journal', 'commands'):
                    clocks = {('observed_at',)} if table == 'records' else {('record', 'observed_at')} if table == 'journal' else {('created_at',), ('history', '*', 'at')}
                    if table == 'commands':
                        command = json.loads(value)
                        expiry = command['command'].get('expires_at', '0001-01-01T00:00:00Z')
                        if expiry == '0001-01-01T00:00:00Z':
                            clocks = dict.fromkeys(clocks, '@clock')
                            clocks[('expires_at',)] = '@ttl:' + str(timestamp_ns(command['expires_at']) - timestamp_ns(command['created_at']))
                    value = normalized_json(value, mapping, clocks)
                if value is None:
                    encoded = b'n'
                elif isinstance(value, bytes):
                    encoded = b'b' + value
                elif isinstance(value, str):
                    encoded = b's' + value.encode()
                else:
                    encoded = b'i' + str(value).encode()
                digest.update(struct.pack('!Q', len(encoded)))
                digest.update(encoded)
            digests.append(digest.digest())
        digest = hashlib.sha256(json.dumps(columns, separators=(',', ':')).encode())
        for row in sorted(digests):
            digest.update(row)
        return digest.hexdigest()


def diff(args):
    with ExitStack() as stack:
        for home in sorted({Path(args.left).resolve(), Path(args.right).resolve()}):
            lock = stack.enter_context(open(home / 'lock', 'rb'))
            fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        return compare(args)


def compare(args):
    results = []
    left_ids, right_ids = identities(args.left), identities(args.right)
    for database, tables in (('state.db', ('records', 'journal', 'journal_commits', 'aliases', 'checkpoints', 'meta')), ('ledger.db', ('commands', 'dedupe', 'meta'))):
        for table in tables:
            left = database_digest(args.left, database, table, left_ids)
            right = database_digest(args.right, database, table, right_ids)
            results.append({'table': database + '/' + table, 'left': left, 'right': right, 'equal': left == right})
    print(json.dumps({'equal': all(r['equal'] for r in results), 'digests': results}, indent=2))
    return int(any(not r['equal'] for r in results))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='verb', required=True)
    p = sub.add_parser('record')
    p.add_argument('--listen', '--socket-tap', dest='listen', required=True)
    p.add_argument('--duration', type=duration)
    p.add_argument('--admission-listen')
    p.add_argument('--admission-upstream')
    p.add_argument('--upstream', required=True)
    p.add_argument('--output', required=True)
    p = sub.add_parser('replay')
    for option in ('recording', 'starting-home', 'output'):
        p.add_argument('--' + option, required=True)
    p.add_argument('--go-binary', '--go', dest='go_binary', required=True)
    p.add_argument('--rust-binary', '--rust', dest='rust_binary', required=True)
    p = sub.add_parser('diff')
    p.add_argument('--left', required=True)
    p.add_argument('--right', required=True)
    args = parser.parse_args()
    try:
        return {'record': record, 'replay': replay, 'diff': diff}[args.verb](args) or 0
    except (ValueError, KeyError, TypeError, OSError, EOFError, sqlite3.Error) as error:
        detail = str(error) if isinstance(error, ValueError) else type(error).__name__
        print('shadow ' + args.verb + ' failed: ' + detail, file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
