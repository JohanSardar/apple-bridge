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
                elif phase['value'] == 'funnel':
                    self.reply(200, {'BackendState': 'Running', 'Self': {'DNSName': 'node.fixture.test.', 'CapMap': {}}})
                else:
                    self.reply(200, {'BackendState': 'Running', 'Self': {'DNSName': 'node.fixture.test.', 'CapMap': {'https': None, 'funnel': None, 'https://tailscale.com/cap/funnel-ports?ports=443,8443,10000': None}}})
            else:
                self.reply(200, configuration)
    def do_POST(self):
        global configuration
        if 'query-feature' in self.path:
            self.rfile.read(int(self.headers.get('Content-Length') or 0))
            return self.reply(200, {'Complete': False, 'URL': 'https://login.tailscale.com/f/funnel?node=fixture'})
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
    assert sorted(fresh) == sorted(['id', 'state', 'backend', 'url', 'last_error', 'pending_link', 'tls_termination', 'certificate_expiry']), fresh
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
            wait(lambda: 'https://login.tailscale.com/f/funnel?node=fixture' in opened.read_text())
            checks.append('funnel_pending_state_and_link_opened')
            assert client.poll() is None
            with lock:
                phase['value'] = 'ready'
            output, errors = client.communicate(timeout=30)
            assert client.returncode == 0, errors
            lines = output.splitlines()
            assert 'Waiting for Tailscale login' in lines and 'https://login.fixture.test/a/login-1' in lines, lines
            assert 'Waiting for Funnel approval' in lines and 'https://login.tailscale.com/f/funnel?node=fixture' in lines, lines
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
            assert failed.returncode != 0 and "Couldn't turn remote access on" in failed.stderr, failed
            document = status_file()
            assert document['state'] == 'error' and document['last_error'].startswith('Remote setup failed') and document['last_error'].endswith('Turn Remote off and try again.'), document
            checks.append('adapter_failure_reports_error_state_and_fails_remote_on')
            subprocess.run([binary, 'remote', 'off', '--via', 'tailscale-app', '--home', str(home)], env=env, capture_output=True, text=True, timeout=30)
            with lock:
                phase['value'] = 'ready'

            def bridge(*arguments, check=True):
                result = subprocess.run([binary, *arguments, '--home', str(home)], env=env, capture_output=True, text=True, timeout=60)
                assert not check or result.returncode == 0, result
                return result

            def proxied(port, path, source, extra=None):
                with lock:
                    target = configuration['Web'][f'node.fixture.test:{port}']['Handlers']['/']['Proxy']
                address, prefix = target.removeprefix('http://').split('/', 1)
                host = 'node.fixture.test' if port == 443 else f'node.fixture.test:{port}'
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

            on = json.loads(bridge('remote', 'enable', '--via', 'tailnet', '--json').stdout)
            assert on['remote']['url'] == 'https://node.fixture.test:8443', on
            with lock:
                assert configuration['TCP']['8443'] == {'HTTPS': True}, configuration
                assert 'node.fixture.test:8443' in configuration['Web'], configuration
                assert not configuration.get('AllowFunnel'), configuration
            document = status_file()
            assert document['routes']['tailnet'] == {'state': 'on', 'url': 'https://node.fixture.test:8443'}, document
            checks.append('tailnet_route_serves_https_without_funnel')

            status, _ = proxied(8443, '/.well-known/oauth-protected-resource/mcp', '100.101.102.103')
            assert status == 200, status
            assert proxied(8443, '/.well-known/oauth-protected-resource/mcp', '203.0.113.9')[0] is None
            assert proxied(8443, '/.well-known/oauth-protected-resource/mcp', '100.101.102.103', {'Tailscale-Funnel-Request': '?1'})[0] is None
            checks.append('tailnet_route_refuses_sources_outside_the_tailnet')

            created = json.loads(bridge('agents', 'create', '--name', 'Grok Bot', '--tailnet', '--scope', 'calendar:read', '--expires-in', '1d', '--json').stdout)
            assert created['address'] == 'https://node.fixture.test:8443/mcp', created
            checks.append('tailnet_key_is_bound_to_the_tailnet_address')

            cloud = json.loads(bridge('remote', 'enable', '--via', 'cloud', '--sources', 'calendar,reminders', '--json').stdout)
            assert cloud['remote']['url'] == 'https://node.fixture.test' and cloud['remote']['backend'] == 'system', cloud
            with lock:
                assert configuration['AllowFunnel'] == {'node.fixture.test:443': True}, configuration
                assert 'node.fixture.test:8443' in configuration['Web'], configuration
            routes = status_file()['routes']
            assert routes['tailnet']['state'] == 'on' and routes['system'] == {'state': 'on', 'url': 'https://node.fixture.test', 'sources': ['calendar', 'reminders']}, routes
            checks.append('cloud_route_opens_funnel_beside_the_tailnet_route')

            status, body = proxied(443, '/.well-known/oauth-protected-resource/mcp', '203.0.113.9')
            scopes = json.loads(body)['scopes_supported']
            assert status == 200 and scopes == ['calendar:read', 'calendar:write', 'reminders:read', 'reminders:write'], (status, scopes)
            status, body = proxied(8443, '/.well-known/oauth-protected-resource/mcp', '100.101.102.103')
            assert len(json.loads(body)['scopes_supported']) > 4
            checks.append('cloud_chats_are_offered_only_the_chosen_services')

            off = json.loads(bridge('remote', 'disable', '--via', 'cloud', '--json').stdout)
            assert off['outcome'] == 'disabled', off
            with lock:
                assert not configuration.get('AllowFunnel') and 'node.fixture.test:443' not in configuration.get('Web', {}), configuration
                assert 'node.fixture.test:8443' in configuration['Web'], configuration
            routes = status_file()['routes']
            assert routes['system']['state'] == 'off' and routes['tailnet']['state'] == 'on', routes
            checks.append('disconnecting_cloud_chats_closes_only_the_public_address')

            bridge('remote', 'disable', '--via', 'tailnet', '--json')
            with lock:
                assert not configuration.get('Web'), configuration
            refused = bridge('agents', 'create', '--name', 'Other', '--tailnet', '--expires-in', '1d', '--json', check=False)
            assert refused.returncode != 0 and 'Tailscale route' in refused.stdout + refused.stderr, refused
            checks.append('tailnet_route_turns_off')
        finally:
            gateway.terminate()
            gateway.wait(timeout=15)

print(json.dumps({'checks': checks}))
