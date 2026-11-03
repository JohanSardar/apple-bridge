import argparse
import base64
from contextlib import ExitStack
import hashlib
import fcntl
import hmac
import json
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
    root = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True)
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
    peers = []
    lock = threading.Lock()
    errors = []
    sequence = 0
    session = 0
    started = time.monotonic_ns()
    with open(path, 'x', opener=lambda p, f: os.open(p, f, 0o600)) as output, socket.socket(socket.AF_UNIX) as server:
        server.bind(str(listen))
        os.chmod(listen, 0o600)
        server.listen(1)
        server.settimeout(.2)
        previous = signal.signal(signal.SIGTERM, lambda *_: stop.set())
        interrupt = signal.signal(signal.SIGINT, lambda *_: stop.set())
        def relay(source, target, direction, number):
            nonlocal sequence
            try:
                while not stop.is_set():
                    raw = receive(source)
                    try:
                        with lock:
                            sequence += 1
                            output.write(json.dumps({'sequence': sequence, 'session': number, 'ns': time.monotonic_ns() - started, 'direction': direction, 'raw': base64.b64encode(raw).decode()}) + '\n')
                            output.flush()
                    except OSError:
                        errors.append('recording disk write failed')
                        stop.set()
                        return
                    target.sendall(raw)
            except (EOFError, OSError):
                pass
            except Exception:
                errors.append('recording failed')
                stop.set()
            finally:
                for conn in (source, target):
                    try:
                        conn.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
                    conn.close()
        try:
            while not stop.is_set():
                try:
                    helper, _ = server.accept()
                except socket.timeout:
                    continue
                gateway = socket.socket(socket.AF_UNIX)
                gateway.connect(args.upstream)
                session += 1
                peers.extend([helper, gateway])
                for worker in [threading.Thread(target=relay, args=(helper, gateway, 'helper', session)), threading.Thread(target=relay, args=(gateway, helper, 'gateway', session))]:
                    workers.append(worker)
                    worker.start()
                workers = [worker for worker in workers if worker.is_alive()]
        finally:
            stop.set()
            for conn in peers:
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


def replay_session(rows, path, secret):
    with connect(path, secret) as conn:
        for row in rows:
            raw = base64.b64decode(row['raw'], validate=True)
            value = frame(raw)
            kind = value.get('t')
            if kind in ('Challenge', 'Authenticate', 'Authenticated', 'Open', 'Ping', 'Pong'):
                continue
            if row['direction'] == 'helper':
                conn.sendall(raw)
            else:
                while True:
                    actual = receive(conn)
                    if frame(actual).get('t') == 'Ping':
                        send(conn, {'t': 'Pong'})
                        continue
                    break
                if frame(actual).get('t') != kind:
                    raise ValueError('replay gateway frame type mismatch at sequence ' + str(row['sequence']))
                if kind in ('Committed', 'Credit') and frame(actual) != value:
                    raise ValueError('replay acknowledgment mismatch at sequence ' + str(row['sequence']))
                if kind not in ('Committed', 'Credit') and actual != raw:
                    raise ValueError('replay requires matching command/query traffic at sequence ' + str(row['sequence']))
        send(conn, {'t': 'Ping'})
        while True:
            kind = frame(receive(conn)).get('t')
            if kind == 'Pong':
                break
            if kind != 'Ping':
                raise ValueError('replay ended with unexpected gateway traffic')
            send(conn, {'t': 'Pong'})


def replay(args):
    destination = private_directory(args.output)
    original = Path(args.starting_home).resolve()
    if destination.is_relative_to(original) or original.is_relative_to(destination):
        raise ValueError('starting home and output must be disjoint')
    starting_lock = open(original / 'lock', 'rb')
    fcntl.flock(starting_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    rows = []
    with open(args.recording) as recording:
        for index, line in enumerate(recording, 1):
            row = json.loads(line)
            raw = base64.b64decode(row['raw'], validate=True)
            if row['sequence'] != index or row['direction'] not in ('helper', 'gateway') or len(raw) < 5 or struct.unpack('!I', raw[:4])[0] != len(raw) - 4:
                raise ValueError('invalid recording at sequence ' + str(index))
            frame(raw)
            rows.append(row)
    pending = set()
    for row in rows:
        value = frame(base64.b64decode(row['raw']))
        if row['direction'] == 'helper' and value.get('t') == 'Capture':
            capture = value.get('capture', value)
            pending.add((row['session'], capture['source'], capture['checkpoint']))
        if row['direction'] == 'gateway' and value.get('t') == 'Committed':
            pending.discard((row['session'], value['source'], value['checkpoint']))
    if pending:
        raise ValueError('recording ends with unacknowledged captures')
    if not rows or not any(frame(base64.b64decode(r['raw'])).get('t') == 'Capture' for r in rows):
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
            try:
                deadline = time.monotonic() + 30
                while not path.exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise ValueError(name + ' blocked: gateway ipc/1 startup unavailable; private log retained')
                    time.sleep(.05)
                secret = (home / 'apple-secret').read_bytes()
                sessions = dict.fromkeys(r['session'] for r in rows)
                for session in sessions:
                    replay_session([r for r in rows if r['session'] == session], path, secret)
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
    return diff(argparse.Namespace(left=destination / 'go', right=destination / 'rust'))


def database_digest(home, database, table):
    path = Path(home).resolve() / database
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
        columns = db.execute('PRAGMA table_info("' + table + '")').fetchall()
        if not columns:
            raise ValueError('required comparison table missing: ' + database + '/' + table)
        rows = db.execute('SELECT * FROM "' + table + '"')
        digests = []
        for row in rows:
            digest = hashlib.sha256()
            for value in row:
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
    for database, tables in (('state.db', ('records', 'journal', 'journal_commits', 'aliases', 'checkpoints', 'meta')), ('ledger.db', ('commands', 'dedupe', 'meta'))):
        for table in tables:
            left = database_digest(args.left, database, table)
            right = database_digest(args.right, database, table)
            results.append({'table': database + '/' + table, 'left': left, 'right': right, 'equal': left == right})
    print(json.dumps({'equal': all(r['equal'] for r in results), 'digests': results}, indent=2))
    return int(any(not r['equal'] for r in results))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='verb', required=True)
    p = sub.add_parser('record')
    p.add_argument('--listen', required=True)
    p.add_argument('--upstream', required=True)
    p.add_argument('--output', required=True)
    p = sub.add_parser('replay')
    for option in ('recording', 'starting-home', 'output', 'go-binary', 'rust-binary'):
        p.add_argument('--' + option, required=True)
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
