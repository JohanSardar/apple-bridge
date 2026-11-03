import http.server
import json
import os
from pathlib import Path
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

binary = str(Path(sys.argv[1]).resolve())
checks = []
phase = {'value': 'login'}
configuration = {}
etag = {'value': 1}
lock = threading.Lock()
ZERO = '0001-01-01T00:00:00Z'


def wait(predicate, seconds=30):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        try:
            result = predicate()
            if result:
                return result
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.05)
    raise AssertionError('timed out')


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args):
        pass
    def reply(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('ETag', str(etag['value']))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        with lock:
            if '/status' in self.path:
                if phase['value'] == 'broken':
                    self.reply(500, {})
                elif phase['value'] == 'login':
                    self.reply(200, {'BackendState': 'NeedsLogin', 'AuthURL': 'https://login.fixture.test/a/login-1'})
                elif phase['value'] == 'funnel':
                    self.reply(200, {'BackendState': 'Running', 'Self': {'DNSName': 'node.fixture.test.', 'CapMap': {}}})
                else:
                    self.reply(200, {'BackendState': 'Running', 'Self': {'DNSName': 'node.fixture.test.', 'CapMap': {'https': None, 'funnel': None, 'https://tailscale.com/cap/funnel-ports?ports=443,8443,10000': None}}})
            elif 'query-feature' in self.path:
                self.reply(200, {'Complete': False, 'URL': 'https://login.fixture.test/f/funnel-1'})
            else:
                self.reply(200, configuration)
    def do_POST(self):
        global configuration
        value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        with lock:
            if self.headers.get('If-Match') != str(etag['value']):
                self.reply(412, {})
            else:
                configuration = value
                etag['value'] += 1
                self.reply(200, {})


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


with tempfile.TemporaryDirectory(prefix='ab-remote-onboarding-') as temp:
    root = Path(temp)
    home = root / 'home'
    socket_path = str(root / 'localapi.sock')
    server = Server(socket_path, Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    fake = root / 'bin'
    fake.mkdir()
    opened = root / 'opened.log'
    for launcher in ('xdg-open', 'open'):
        script = fake / launcher
        script.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "' + str(opened) + '"\n')
        script.chmod(0o755)
    env = dict(os.environ, PATH=str(fake) + os.pathsep + os.environ['PATH'], APPLE_BRIDGE_SECRET_STORE='file', APPLE_BRIDGE_TAILSCALE_SOCKET=socket_path, APPLE_BRIDGE_REMOTE_RECHECK_MS='200')

    def status_file():
        return json.loads((home / 'remote/status.json').read_text())

    fresh = json.loads(subprocess.check_output([binary, 'remote', 'status', '--home', str(home)], env=env, text=True))
    assert list(fresh) == ['id', 'state', 'backend', 'url', 'last_error', 'pending_link', 'tls_termination', 'certificate_expiry'], fresh
    assert fresh['state'] == 'off' and fresh['certificate_expiry'] == ZERO and fresh['url'] == ''
    checks.append('remote_status_returns_the_status_document')

    with (root / 'gateway.log').open('wb') as log:
        gateway = subprocess.Popen([binary, 'start', '--role', 'gateway', '--home', str(home), '--api-addr', '127.0.0.1:0', '--peer-addr', '127.0.0.1:0'], env=env, stdout=log, stderr=log)
        try:
            wait(lambda: (home / 'runtime.json').exists())
            client = subprocess.Popen([binary, 'remote', 'on', '--via', 'tailscale-app', '--home', str(home)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            pending = wait(lambda: status_file() if status_file()['state'] == 'login_pending' else None)
            assert pending['message'] == 'Waiting for Tailscale login' and pending['pending_link'] == 'login pending' and pending['last_error'] == '' and pending['url'] == '', pending
            wait(lambda: 'https://login.fixture.test/a/login-1' in opened.read_text())
            checks.append('login_pending_state_and_link_opened')
            with lock:
                phase['value'] = 'funnel'
            pending = wait(lambda: status_file() if status_file()['state'] == 'funnel_pending' else None)
            assert pending['message'] == 'Waiting for Funnel approval' and pending['pending_link'] == 'Funnel approval pending', pending
            wait(lambda: 'https://login.fixture.test/f/funnel-1' in opened.read_text())
            checks.append('funnel_pending_state_and_link_opened')
            assert client.poll() is None
            with lock:
                phase['value'] = 'ready'
            output, errors = client.communicate(timeout=30)
            assert client.returncode == 0, errors
            lines = output.splitlines()
            assert 'Waiting for Tailscale login' in lines and 'https://login.fixture.test/a/login-1' in lines, lines
            assert 'Waiting for Funnel approval' in lines and 'https://login.fixture.test/f/funnel-1' in lines, lines
            assert lines[-2:] == ['https://node.fixture.test', 'https://node.fixture.test/mcp'], lines
            document = json.loads(subprocess.check_output([binary, 'remote', 'status', '--home', str(home)], env=env, text=True))
            assert document['state'] == 'on' and document['url'] == 'https://node.fixture.test' and document['backend'] == 'system' and document['last_error'] == '' and document['pending_link'] == '', document
            checks.append('remote_on_waits_and_prints_url_and_mcp')
            off = subprocess.run([binary, 'remote', 'off', '--via', 'tailscale-app', '--home', str(home)], env=env, capture_output=True, text=True, timeout=30)
            assert off.returncode == 0 and off.stdout.strip() == 'off', off
            checks.append('remote_off_waits_for_off')
            with lock:
                phase['value'] = 'broken'
            failed = subprocess.run([binary, 'remote', 'on', '--via', 'tailscale-app', '--home', str(home)], env=env, capture_output=True, text=True, timeout=30)
            assert failed.returncode != 0 and failed.stderr.strip() == 'remote: Remote setup failed. Turn Remote off and try again.', failed
            document = status_file()
            assert document['state'] == 'error' and document['last_error'] == 'Remote setup failed. Turn Remote off and try again.', document
            checks.append('adapter_failure_reports_error_state_and_fails_remote_on')
        finally:
            gateway.terminate()
            gateway.wait(timeout=15)

print(json.dumps({'checks': checks}))
