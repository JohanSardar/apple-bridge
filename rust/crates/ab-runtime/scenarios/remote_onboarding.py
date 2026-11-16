import http.client
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
                else:
                    self.reply(200, {'BackendState': 'Running', 'Self': {'DNSName': 'node.fixture.test.', 'CapMap': {'https': None}}})
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
    env = dict(os.environ, PATH=str(fake) + os.pathsep + os.environ['PATH'], APPLE_BRIDGE_SECRET_STORE='file', SSH_CONNECTION='1', APPLE_BRIDGE_TAILSCALE_SOCKET=socket_path, APPLE_BRIDGE_REMOTE_RECHECK_MS='200')

    def status_file():
        return json.loads((home / 'remote/status.json').read_text())

    fresh = json.loads(subprocess.check_output([binary, 'tailscale', '--off', '--json', '--home', str(home)], env=env, text=True))
    assert fresh == {'outcome': 'unchanged'}, fresh
    checks.append('tailscale_off_on_a_fresh_home_is_unchanged')

    def turn_on():
        return subprocess.Popen([binary, 'tailscale', '--home', str(home)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    with (root / 'gateway.log').open('wb') as log:
        gateway = subprocess.Popen([binary, 'start', '--role', 'gateway', '--home', str(home), '--api-addr', '127.0.0.1:0', '--peer-addr', '127.0.0.1:0'], env=env, stdout=log, stderr=log)
        try:
            wait(lambda: (home / 'runtime.json').exists())

            def bridge(*arguments, check=True):
                result = subprocess.run([binary, *arguments, '--home', str(home)], env=env, capture_output=True, text=True, timeout=60)
                assert not check or result.returncode == 0, result
                return result

            def proxied(port, path, source, extra=None):
                with lock:
                    target = configuration['Web'][f'node.fixture.test:{port}']['Handlers']['/']['Proxy']
                address, prefix = target.removeprefix('http://').split('/', 1)
                host = f'node.fixture.test:{port}'
                head = {'Host': host, 'X-Forwarded-Host': host, 'X-Forwarded-Proto': 'https', 'X-Forwarded-For': source, **(extra or {})}
                connection = http.client.HTTPConnection(*address.split(':'), timeout=10)
                try:
                    connection.request('GET', '/' + prefix + path, headers=head)
                    response = connection.getresponse()
                    return response.status, response.read()
                except (ConnectionError, http.client.HTTPException):
                    return None, b''
                finally:
                    connection.close()

            client = turn_on()
            pending = wait(lambda: status_file() if status_file()['state'] == 'login_pending' else None)
            assert pending['message'] == 'Waiting for Tailscale login' and pending['pending_link'] == 'login pending' and pending['last_error'] == '' and pending['url'] == '', pending
            assert client.poll() is None
            checks.append('login_pending_state')
            with lock:
                phase['value'] = 'ready'
            output, errors = client.communicate(timeout=30)
            assert client.returncode == 0, errors
            assert 'Waiting for Tailscale login' in errors and 'https://login.fixture.test/a/login-1' in errors, errors
            assert 'https://login.fixture.test/a/login-1' in output.splitlines(), output
            assert output.splitlines()[-1:] == ['https://node.fixture.test:8443/mcp'], output
            assert not opened.exists(), opened.read_text()
            document = wait(lambda: status_file() if status_file()['state'] == 'on' else None)
            assert document['url'] == 'https://node.fixture.test:8443' and document['last_error'] == '' and document['pending_link'] == '', document
            with lock:
                assert configuration['TCP']['8443'] == {'HTTPS': True}, configuration
                assert 'node.fixture.test:8443' in configuration['Web'], configuration
                assert not configuration.get('AllowFunnel'), configuration
            assert document['routes']['tailnet'] == {'state': 'on', 'url': 'https://node.fixture.test:8443'}, document
            checks.append('tailscale_waits_for_login_then_serves_https_on_the_tailnet')

            status, _ = proxied(8443, '/.well-known/oauth-protected-resource/mcp', '100.101.102.103')
            assert status == 200, status
            assert proxied(8443, '/.well-known/oauth-protected-resource/mcp', '203.0.113.9')[0] is None
            assert proxied(8443, '/.well-known/oauth-protected-resource/mcp', '100.101.102.103', {'Tailscale-Funnel-Request': '?1'})[0] is None
            checks.append('tailnet_route_refuses_sources_outside_the_tailnet')

            created = json.loads(bridge('agents', 'create', '--name', 'Grok Bot', '--tailnet', '--scope', 'calendar:read', '--expires-in', '1d', '--json').stdout)
            assert created['address'] == 'https://node.fixture.test:8443/mcp', created
            checks.append('tailnet_key_is_bound_to_the_tailnet_address')

            off = json.loads(bridge('tailscale', '--off', '--json').stdout)
            assert off == {'outcome': 'off'}, off
            with lock:
                assert not configuration.get('Web'), configuration
            refused = bridge('agents', 'create', '--name', 'Other', '--tailnet', '--expires-in', '1d', '--json', check=False)
            assert refused.returncode != 0 and 'Tailscale route' in refused.stdout + refused.stderr, refused
            checks.append('tailscale_off_removes_the_route')

            with lock:
                phase['value'] = 'broken'
            client = turn_on()
            output, errors = client.communicate(timeout=30)
            assert client.returncode != 0 and "Couldn't turn Tailscale access on" in errors, errors
            document = wait(lambda: status_file() if status_file()['state'] == 'error' else None)
            assert document['last_error'].startswith('Tailscale setup failed') and document['last_error'].endswith('Run bridge tailscale --off, then try again.'), document
            checks.append('adapter_failure_reports_error_state')
            bridge('tailscale', '--off')
        finally:
            gateway.terminate()
            gateway.wait(timeout=15)

print(json.dumps({'checks': checks}))
