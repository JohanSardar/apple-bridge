import argparse
import concurrent.futures
import http.client
import http.server
import base64
import calendar
from isolation import require_isolation, require_home
from contextlib import ExitStack
from datetime import datetime, timezone
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


INGRESS_PREFIXES = ('/v1/', '/mcp')


def wall_clock():
    now = time.time_ns()
    return datetime.fromtimestamp(now // 1000000000, timezone.utc).strftime('%Y-%m-%dT%H:%M:%S') + '.%09dZ' % (now % 1000000000)


def ingress_kind(method, path, headers):
    label = {key.lower(): value for key, value in headers.items()}.get('x-ingress')
    if label in ('cli', 'feed', 'iphone'):
        return label
    if path == '/mcp' or path.startswith('/mcp/'):
        return 'mcp'
    return 'rest'


def record(args):
    directory = private_directory(args.output)
    if args.delegated_log and Path(args.delegated_log).exists():
        raise ValueError('delegated feed log must not exist before recording')
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
            output.write(json.dumps(dict(sequence=sequence, session=number, ns=time.monotonic_ns() - started, wall=wall_clock(), direction=direction, raw=base64.b64encode(raw).decode(), **metadata)) + '\n')
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
        observer_server = None
        if args.iphone_observer:
            observer_path = Path(args.iphone_observer).resolve()
            if observer_path.exists():
                raise ValueError('iPhone observer socket already exists')
            observer_server = socket.socket(socket.AF_UNIX)
            observer_server.bind(str(observer_path))
            os.chmod(observer_path, 0o600)
            observer_server.listen(16)
            observer_server.settimeout(.2)
            def observe(conn):
                with conn:
                    conn.settimeout(30)
                    line = b''
                    while not line.endswith(b'\n'):
                        chunk = conn.recv(1 << 20)
                        if not chunk:
                            return
                        line += chunk
                    event = json.loads(line)
                    if event['event'] == 'request':
                        body = json.dumps({'device_id': event['device_id'], 'at': event['at'], 'payload': event.get('payload') or ''}).encode()
                        number = append('admission', body, session, method='POST', path=event['path'], ingress='iphone', headers={'Content-Type': 'application/json', 'X-Ingress': 'iphone'})
                    else:
                        number = append('response', base64.b64decode(event.get('body') or ''), session, request=event['request'], status=event['status'])
                    conn.sendall(json.dumps({'sequence': number}).encode() + b'\n')
            def observe_loop():
                while not stop.is_set():
                    try:
                        conn, _ = observer_server.accept()
                    except socket.timeout:
                        continue
                    except OSError:
                        return
                    def guarded(connection=conn):
                        try:
                            observe(connection)
                        except (OSError, ValueError, KeyError):
                            errors.append('iPhone observer failed')
                            stop.set()
                    threading.Thread(target=guarded, daemon=True).start()
            threading.Thread(target=observe_loop, daemon=True).start()
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
                def handle_ingress(self):
                    path = self.path
                    if self.headers.get('Transfer-Encoding') or not any(path == prefix.rstrip('/') or path.startswith(prefix) for prefix in INGRESS_PREFIXES):
                        self.send_error(400)
                        return
                    size = int(self.headers.get('Content-Length', '0'))
                    if not 0 <= size <= MAX_FRAME or self.command == 'POST' and size == 0:
                        self.send_error(400)
                        return
                    body = self.rfile.read(size) if size else b''
                    if len(body) != size:
                        self.send_error(400)
                        return
                    headers = {key: value for key, value in self.headers.items() if key.lower() not in ('host', 'connection')}
                    try:
                        number = session
                        request = append('admission', body, number, method=self.command, path=path, ingress=ingress_kind(self.command, path, headers), headers=headers)
                        conn = http.client.HTTPConnection(args.admission_upstream, timeout=60)
                        try:
                            conn.request(self.command, path, body or None, headers)
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
                do_POST = do_GET = do_PUT = do_DELETE = handle_ingress
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
            if observer_server:
                observer_server.close()
                observer_path.unlink(missing_ok=True)
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
    if args.delegated_log:
        sequence = merge_delegated(path, Path(args.delegated_log))
    print(json.dumps({'recorded_frames': sequence, 'sessions': session}))


def merge_delegated(path, log):
    if not log.exists():
        return sum(1 for _ in open(path))
    rows = [json.loads(line) for line in open(path)]
    additions = []
    for index, line in enumerate(open(log)):
        entry = json.loads(line)
        if entry['method'] not in ('submit', 'status', 'list') or not isinstance(entry['delegation'], dict):
            raise ValueError('invalid delegated feed observation')
        body = base64.b64decode(entry['body'], validate=True)
        json.loads(body)
        outcome = dict(entry['outcome'])
        status = outcome.pop('status')
        additions.append((timestamp_ns(entry['start']), 0, index, dict(direction='admission', raw=entry['body'], wall=entry['start'], method='POST', path='/v1/commands', ingress='feed', headers={}, delegation=entry['delegation'], feed_index=index)))
        additions.append((timestamp_ns(entry['end']), 1, index, dict(direction='response', raw=base64.b64encode(json.dumps(outcome, sort_keys=True).encode()).decode(), wall=entry['end'], status=status, feed_index=index)))
    merged = list(rows)
    for _, _, _, row in sorted(additions, key=lambda item: item[:3]):
        wall = timestamp_ns(row['wall'])
        position = len(merged)
        for offset, existing in enumerate(merged):
            if timestamp_ns(existing['wall']) > wall:
                position = offset
                break
        merged.insert(position, row)
    requests = {}
    session = 1
    for number, row in enumerate(merged, 1):
        if 'feed_index' in row:
            row['session'] = session
            row['ns'] = merged[number - 2]['ns'] if number > 1 else 0
            if row['direction'] == 'admission':
                requests[('feed', row['feed_index'])] = number
            else:
                row['request'] = requests.pop(('feed', row['feed_index']))
            del row['feed_index']
        else:
            session = row['session']
            if row['direction'] == 'admission':
                requests[row['sequence']] = number
            elif row['direction'] == 'response':
                row['request'] = requests.pop(row['request'])
        row['sequence'] = number
    staged = path.with_suffix('.merged')
    with open(staged, 'x', opener=lambda p, f: os.open(p, f, 0o600)) as output:
        for row in merged:
            output.write(json.dumps(row) + '\n')
    os.replace(staged, path)
    return len(merged)


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


IPHONE_DEVICE = None


def iphone_request(device, path, at, payload):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(device['signing_seed']))
    signature = key.sign(b'apple-bridge-iphone/1\0' + path.encode() + b'\0' + str(at).encode() + b'\0' + payload)
    return json.dumps({'device_id': device['device_id'], 'at': at, 'payload': base64.b64encode(payload).decode(), 'signature': base64.b64encode(signature).decode()}).encode()


def wall_seconds(wall):
    return calendar.timegm(time.strptime(wall[:19], '%Y-%m-%dT%H:%M:%S'))


def admit(row, address, at=None):
    conn = http.client.HTTPConnection(address, timeout=60)
    try:
        body = base64.b64decode(row['raw'], validate=True)
        headers = row['headers']
        if row.get('ingress') == 'iphone':
            if IPHONE_DEVICE is None:
                raise ValueError('recording contains iPhone ingress; pass --iphone-device')
            recorded = json.loads(body)
            body = iphone_request(IPHONE_DEVICE, row['path'], int(time.time()) if at is None else at, base64.b64decode(recorded['payload']))
            headers = {'Content-Type': 'application/json'}
        conn.request(row.get('method', 'POST'), row['path'], body or None, headers)
        result = conn.getresponse()
        raw = result.read()
        try:
            response = json.loads(raw)
        except ValueError:
            response = None
        return result.status, response
    finally:
        conn.close()


def feed_admit(row, address, peer_home, binary, env):
    body = base64.b64decode(row['raw'], validate=True)
    request = {'body': row['raw'], 'method': json.loads(body)['method'], 'delegation': row['delegation']}
    result = subprocess.run([binary, 'shadow-feed', '--peer-home', str(peer_home), '--address', address], input=json.dumps(request), capture_output=True, text=True, env=env, timeout=90)
    if result.returncode:
        raise ValueError('replay delegated feed request failed: ' + result.stderr.strip())
    outcome = json.loads(result.stdout)['outcome']
    return outcome['status'], outcome


def replay_session(rows, path, secret, api_address, home, clock=None, feed=None):
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        conn = connect(path, secret)
        admissions = {}
        sent = {}
        source_states = {}
        def drain():
            nonlocal source_states
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
        names = [home / name for name in ('state.db', 'state.db-wal', 'ledger.db', 'ledger.db-wal')]
        def marks():
            return [(entry.stat().st_mtime_ns, entry.stat().st_size) if entry.exists() else None for entry in names]
        def settle():
            deadline = time.monotonic() + 15
            quiet = time.monotonic()
            previous = marks()
            while time.monotonic() - quiet < .4:
                time.sleep(.025)
                current = marks()
                if current != previous:
                    previous, quiet = current, time.monotonic()
                if time.monotonic() > deadline:
                    raise ValueError('replay gateway did not settle')
        def persisting(before, admission):
            deadline = time.monotonic() + 5
            while marks() == before and not admission.done() and time.monotonic() < deadline:
                time.sleep(.005)
        for row in rows:
            stepped = bool(clock and row.get('wall') and row['direction'] != 'response')
            if stepped and row['direction'] != 'gateway':
                settle()
                clock(row['wall'])
            if row['direction'] == 'admission':
                before = marks()
                if row.get('ingress') == 'feed':
                    if feed is None:
                        raise ValueError('recording has delegated feed ingress; replay needs --feed-peer-home')
                    admissions[row['sequence']] = pool.submit(feed, row)
                else:
                    admissions[row['sequence']] = pool.submit(admit, row, api_address, wall_seconds(row['wall']) if clock and row.get('wall') else None)
                sent[row['sequence']] = row
                persisting(before, admissions[row['sequence']])
                continue
            if row['direction'] == 'response':
                status, response = admissions.pop(row['request']).result(timeout=60)
                recorded = recorded_response(row)
                if not response_matches(sent.pop(row['request']), row['status'], status, response, recorded):
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
                    drain()
                if kind == 'Ready':
                    for source in value.get('coverage') or {}:
                        source_states[source] = 'ready'
                    drain()
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
                if stepped:
                    settle()
                    clock(row['wall'])
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
        drain()
        return conn


def recorded_response(row):
    try:
        return json.loads(base64.b64decode(row['raw'], validate=True))
    except ValueError:
        return None


def response_matches(row, expected, status, response, recorded):
    if status != expected:
        return False
    if recorded is None or response is None:
        return recorded is response
    if row['path'] == '/v1/commands':
        return isinstance(response, dict) and isinstance(recorded, dict) and {key: value for key, value in response.items() if key != 'status'} == recorded
    if row['path'].startswith('/v1/op/') and row['path'] != '/v1/op/records.query':
        return isinstance(response, dict) and isinstance(recorded, dict) and all(response.get(key) == recorded.get(key) for key in ('state', 'disposition', 'error'))
    if isinstance(recorded, dict) and 'jsonrpc' in recorded:
        return isinstance(response, dict) and response.get('id') == recorded.get('id') and bool(response.get('error')) == bool(recorded.get('error')) and bool((response.get('result') or {}).get('isError')) == bool((recorded.get('result') or {}).get('isError'))
    return type(response) is type(recorded) and (not isinstance(recorded, dict) or sorted(response) == sorted(recorded))


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
            elif row['direction'] == 'admission':
                if not any(row['path'] == prefix.rstrip('/') or row['path'].startswith(prefix) for prefix in INGRESS_PREFIXES) or not isinstance(row['headers'], dict) or row.get('method', 'POST') not in ('GET', 'POST', 'PUT', 'DELETE'):
                    raise ValueError('invalid admission metadata')
                if row.get('method', 'POST') == 'POST':
                    json.loads(raw)
                if row.get('ingress') == 'feed':
                    delegation = row.get('delegation')
                    if row['path'] != '/v1/commands' or json.loads(raw).get('method') not in ('submit', 'status', 'list') or not isinstance(delegation, dict) or not isinstance(delegation.get('local_principal'), str) or not isinstance(delegation.get('local_grants'), list):
                        raise ValueError('invalid delegated feed admission')
            elif len(raw) < 5 or len(raw) > MAX_FRAME + 4 or struct.unpack('!I', raw[:4])[0] != len(raw) - 4:
                raise ValueError('invalid recording at sequence ' + str(index))
            else:
                frame(raw)
            yield row


def replay(args):
    require_isolation()
    global IPHONE_DEVICE
    IPHONE_DEVICE = json.loads(Path(args.iphone_device).read_text()) if args.iphone_device else None
    require_home(args.starting_home)
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
    feeds = any(row.get('ingress') == 'feed' for row in recording_rows(args.recording) if row['direction'] == 'admission')
    if feeds and not args.feed_peer_home:
        raise ValueError('recording has delegated feed ingress; replay needs --feed-peer-home')
    if feeds:
        require_home(args.feed_peer_home)
        for entry in Path(args.feed_peer_home).resolve().rglob('*'):
            if entry.is_symlink():
                raise ValueError('feed peer home contains symlinks')
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
        env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file', APPLE_BRIDGE_LAB_IPHONE_PLAINTEXT='1')
        env.pop('APPLE_BRIDGE_LAB_IPHONE_OBSERVER', None)
        if args.decisions:
            decisions = Path(args.decisions).resolve()
            manifest = json.loads(decisions.read_bytes())
            timestamp_ns(manifest['now'])
            if not isinstance(manifest.get('command_ids', []), list) or any(not isinstance(i, str) or not i for i in manifest.get('command_ids', [])):
                raise ValueError('invalid replay command id decisions')
            if not isinstance(manifest['aliases'], dict) or any(not isinstance(k, str) or not isinstance(v, str) or not v for k, v in manifest['aliases'].items()):
                raise ValueError('invalid replay alias decisions')
            decision_copy = destination / (name + '-decisions.json')
            decision_copy.write_bytes(decisions.read_bytes())
            os.chmod(decision_copy, 0o600)
            env['APPLE_BRIDGE_LAB_REPLAY'] = str(decision_copy)
            if manifest.get('clock') == 'recorded':
                def clock(wall, target=decision_copy, template=manifest):
                    timestamp_ns(wall)
                    staged = target.with_suffix('.next')
                    staged.write_text(json.dumps(dict(template, now=wall)))
                    os.chmod(staged, 0o600)
                    os.replace(staged, target)
            else:
                clock = None
        else:
            clock = None
            env.pop('APPLE_BRIDGE_LAB_REPLAY', None)
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
                configuration = json.loads(runtime.read_text())
                address = configuration['api_address']
                feed = None
                if feeds:
                    peer = destination / (name + '-peer')
                    shutil.copytree(Path(args.feed_peer_home).resolve(), peer, ignore=shutil.ignore_patterns('*.sock', 'lock', 'pid'))
                    feed_env = {key: value for key, value in env.items() if key != 'APPLE_BRIDGE_LAB_REPLAY'}
                    feed = lambda row, peer=peer, peer_address=configuration['peer_address']: feed_admit(row, peer_address, peer, str(Path(args.lab_binary).resolve()), feed_env)
                for session, rows in itertools.groupby(recording_rows(args.recording), key=lambda row: row['session']):
                    if active_conn:
                        active_conn.close()
                    active_conn = replay_session(rows, path, secret, address, home, clock, feed)
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                if active_conn:
                    active_conn.close()
    return diff(argparse.Namespace(left=destination / 'go', right=destination / 'rust', exact=args.exact))


def decisions(args):
    first = None
    for row in recording_rows(args.recording):
        first = row.get('wall')
        break
    if not first:
        raise ValueError('recording has no wall clock; record again with this tool version')
    timestamp_ns(first)
    path = Path(args.home).resolve() / 'state.db'
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
        aliases = {'\0'.join(row[:4]): row[4] for row in db.execute('SELECT source,generation,collection,native_key,record_id FROM aliases')}
    command_ids = generated_command_ids(args)
    output = Path(args.output).resolve()
    if output.exists() or output.parent.stat().st_mode & 0o077:
        raise ValueError('decision manifest must be new in an owner-only directory')
    with open(output, 'x', opener=lambda p, f: os.open(p, f, 0o600)) as manifest:
        manifest.write(json.dumps({'now': first, 'clock': 'recorded', 'aliases': aliases, 'command_ids': command_ids}))
    print(json.dumps({'aliases': len(aliases), 'command_ids': len(command_ids), 'clock': 'recorded'}))


def generated_command_ids(args):
    def ledger(home):
        path = Path(home).resolve() / 'ledger.db'
        with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
            return [(row[0], timestamp_ns(json.loads(row[1])['created_at'])) for row in db.execute('SELECT id,payload FROM commands ORDER BY rowid')]
    known = {identifier for identifier, _ in ledger(args.starting_home)} if args.starting_home else set()
    supplied = b''.join(base64.b64decode(row['raw']) for row in recording_rows(args.recording) if row['direction'] == 'admission')
    pending = [(created, identifier) for identifier, created in ledger(args.home) if identifier not in known and identifier.encode() not in supplied]
    return [identifier for _, identifier in sorted(pending, key=lambda item: item[0])]


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


def database_digest(home, database, table, mapping, exact=False):
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
                if not exact and database == 'state.db' and ((table == 'records' and name == 'observed_at') or (table == 'journal' and name == 'committed_at')):
                    value = '@clock'
                elif not exact and name in ('id', 'record_id') and isinstance(value, str):
                    value = mapping.get(value, value)
                elif not exact and name == 'payload' and table in ('records', 'journal', 'commands'):
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
    exact = getattr(args, 'exact', False)
    left_ids, right_ids = ({}, {}) if exact else (identities(args.left), identities(args.right))
    for database, tables in (('state.db', ('records', 'journal', 'journal_commits', 'aliases', 'checkpoints', 'meta')), ('ledger.db', ('commands', 'dedupe', 'meta'))):
        for table in tables:
            left = database_digest(args.left, database, table, left_ids, exact)
            right = database_digest(args.right, database, table, right_ids, exact)
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
    p.add_argument('--iphone-observer')
    p.add_argument('--upstream', required=True)
    p.add_argument('--output', required=True)
    p.add_argument('--delegated-log')
    p = sub.add_parser('replay')
    for option in ('recording', 'starting-home', 'output'):
        p.add_argument('--' + option, required=True)
    p.add_argument('--decisions')
    p.add_argument('--iphone-device')
    p.add_argument('--exact', action='store_true')
    p.add_argument('--go-binary', '--go', dest='go_binary', required=True)
    p.add_argument('--rust-binary', '--rust', dest='rust_binary', required=True)
    p.add_argument('--feed-peer-home')
    p.add_argument('--lab-binary', default='bin/bridge-lab')
    p = sub.add_parser('decisions')
    for option in ('recording', 'home', 'output'):
        p.add_argument('--' + option, required=True)
    p.add_argument('--starting-home')
    p = sub.add_parser('diff')
    p.add_argument('--exact', action='store_true')
    p.add_argument('--left', required=True)
    p.add_argument('--right', required=True)
    args = parser.parse_args()
    try:
        return {'record': record, 'replay': replay, 'diff': diff, 'decisions': decisions}[args.verb](args) or 0
    except (ValueError, KeyError, TypeError, OSError, EOFError, sqlite3.Error) as error:
        detail = str(error) if isinstance(error, ValueError) else type(error).__name__
        print('shadow ' + args.verb + ' failed: ' + detail, file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
