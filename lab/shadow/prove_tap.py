import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time

from shadow import private_directory


def read_all(conn):
    result = bytearray()
    while True:
        data = conn.recv(8192)
        if not data:
            return bytes(result)
        result.extend(data)


def packet(payload):
    return struct.pack('!I', len(payload)) + payload


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    os.umask(0o077)
    root = private_directory(args.output)
    if list(root.iterdir()):
        raise ValueError('proof directory must be empty')
    request = packet(b'{ "t": "Capture", "data": "\\u003c&\\u003e" }\n') + packet(bytes(range(256)) * 2048)
    response = packet(b' {"t":"Committed","checkpoint":"synthetic"}\n') + packet(bytes(reversed(range(256))) * 2048)
    received = []
    failures = []
    with socket.socket(socket.AF_UNIX) as upstream:
        upstream.bind(str(root / 'upstream.sock'))
        upstream.listen()
        upstream.settimeout(15)
        def gateway():
            try:
                with upstream.accept()[0] as peer:
                    peer.settimeout(15)
                    received.append(read_all(peer))
                    for offset in range(0, len(response), 997):
                        peer.sendall(response[offset:offset + 997])
                    peer.shutdown(socket.SHUT_WR)
            except Exception as error:
                failures.append(type(error).__name__)
        worker = threading.Thread(target=gateway)
        worker.start()
        process = subprocess.Popen([sys.executable, str(Path(__file__).with_name('shadow.py')), 'record', '--socket-tap', str(root / 'tap.sock'), '--upstream', str(root / 'upstream.sock'), '--output', str(root / 'recording'), '--duration', '3s'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 10
            while not (root / 'tap.sock').exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('tap startup failed')
                time.sleep(.01)
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(15)
                client.connect(str(root / 'tap.sock'))
                for offset in range(0, len(request), 791):
                    client.sendall(request[offset:offset + 791])
                client.shutdown(socket.SHUT_WR)
                returned = read_all(client)
            with (root / 'recording/ipc.shadow').open('rb') as recording:
                try:
                    fcntl.flock(recording, fcntl.LOCK_SH | fcntl.LOCK_NB)
                except BlockingIOError:
                    pass
                else:
                    raise RuntimeError('active recording was not exclusively locked')
            worker.join(timeout=15)
            if worker.is_alive() or failures or received != [request] or returned != response:
                raise RuntimeError('tap changed bytes or lost half-close response')
            output, error = process.communicate(timeout=10)
            if process.returncode:
                raise RuntimeError('tap failed')
            rows = [json.loads(line) for line in (root / 'recording/ipc.shadow').read_text().splitlines()]
            for direction, expected in [('helper', request), ('gateway', response)]:
                actual = b''.join(base64.b64decode(row['raw'], validate=True) for row in rows if row['direction'] == direction)
                if actual != expected:
                    raise RuntimeError('recording changed wire bytes')
            if [row['sequence'] for row in rows] != list(range(1, len(rows) + 1)):
                raise RuntimeError('recording order invalid')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
    print(json.dumps({'bidirectional_bytes_identical': True, 'recording_bytes_identical': True, 'fragmented_frames': True, 'half_close_response_preserved': True, 'duration_exit': True, 'active_recording_locked': True, 'bytes_each_direction': [len(request), len(response)]}))


if __name__ == '__main__':
    main()
