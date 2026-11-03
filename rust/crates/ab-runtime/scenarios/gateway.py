import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
root = Path(__file__).resolve().parents[4]
env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file')
checks = []


def exact(stream, size):
    result = b''
    while len(result) < size:
        chunk = stream.recv(size - len(result))
        assert chunk, 'IPC disconnected'
        result += chunk
    return result


def receive(stream):
    while True:
        frame = json.loads(exact(stream, struct.unpack('!I', exact(stream, 4))[0]))
        if frame['t'] != 'Ping':
            return frame
        send(stream, {'t': 'Pong'})


def send(stream, value):
    data = json.dumps(value, separators=(',', ':')).encode()
    stream.sendall(struct.pack('!I', len(data)) + data)


with tempfile.TemporaryDirectory(prefix='ab-gateway-assembly-') as temp:
    home = Path(temp)
    for attempt in range(2):
        (home / 'runtime.json').unlink(missing_ok=True)
        with (home / 'gateway.log').open('wb') as log:
            process = subprocess.Popen([binary, 'start', '--home', temp, '--role', 'gateway', '--api-addr', '127.0.0.1:0', '--peer-addr', '127.0.0.1:0'], env=env, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 15
                while not (home / 'runtime.json').exists() or not (home / 'run/peer.sock').exists():
                    assert process.poll() is None, (home / 'gateway.log').read_text()
                    assert time.monotonic() < deadline, 'gateway readiness timeout'
                    time.sleep(.02)
                config = json.loads((home / 'runtime.json').read_text())
                checks.append('default_socket_ready')
                token = subprocess.check_output([binary, 'token', 'create', '--home', temp, '--scope', 'contacts:read,system:read'], env=env, text=True).strip()
                connection = http.client.HTTPConnection(config['api_address'], timeout=5)
                connection.request('GET', '/', headers={'Host': config['api_address']})
                response = connection.getresponse()
                assert response.status in (200, 302, 303), (response.status, response.read())
                response.read()
                checks.append('web_outside_bearer_guard')
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.settimeout(5)
                    stream.connect(config['socket'])
                    challenge = receive(stream)
                    nonce = os.urandom(32).hex()
                    secret = (home / 'apple-secret').read_bytes()
                    def proof(role):
                        return hmac.new(secret, ('apple-bridge-ipc/1\0' + role + '\0' + challenge['nonce'] + '\0' + nonce).encode(), hashlib.sha256).hexdigest()
                    send(stream, {'t':'Authenticate', 'nonce':nonce, 'proof':proof('helper')})
                    assert receive(stream)['proof'] == proof('gateway')
                    opened = receive(stream)
                    assert opened['t'] == 'Open'
                    if attempt:
                        assert opened['owner_fence'] > fence
                    fence = opened['owner_fence']
                    send(stream, {'t':'Ready','protocol':'ipc/1','helper_build':'synthetic'})
                    frame = json.loads(json.loads((root / 'contracts/golden/ipc.json').read_text())['frames'][2]['json'])
                    send(stream, frame)
                    committed = receive(stream)
                    assert committed['t'] == 'Committed', committed
                    checks.append('authenticated_capture_committed')
                    connection.request('POST', '/v1/op/records.query', json.dumps({'collection':'contacts.contact'}), {'Authorization':'Bearer ' + token, 'Host':config['api_address'], 'Content-Type':'application/json'})
                    response = connection.getresponse()
                    data = response.read()
                    assert response.status == 200, (response.status, data)
                    result = json.loads(data)
                    assert len(result['records']) == 1, result
                    if attempt:
                        assert result['records'] == records
                        assert result['cursor'] == cursor
                        checks.append('restart_capture_deduplicated')
                    records, cursor = result['records'], result['cursor']
                    checks.append('capture_visible_through_gateway_service')
                connection.close()
            finally:
                process.terminate()
                assert process.wait(timeout=15) == 0, (home / 'gateway.log').read_text()
                assert not (home / 'run/peer.sock').exists()
                checks.append('shutdown_socket_cleanup')
print(json.dumps({'impl':'rust','checks':checks,'passed':True}))
