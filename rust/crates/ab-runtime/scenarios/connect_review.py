import base64
import hashlib
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
import unittest
from urllib.parse import parse_qs, urlencode, urlsplit

BINARY = str(Path(sys.argv.pop(1)).resolve())
HOST = 'node.fixture.test'
ORIGIN = 'https://' + HOST
CALLBACK = 'https://connector.example.test/callback'


def wait(predicate):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.05)
    raise AssertionError('fixture did not become ready')


class LocalAPI(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('ETag', str(self.server.etag))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if '/status' in self.path:
            self.reply({'BackendState': 'Running', 'Self': {
                'DNSName': HOST + '.', 'CapMap': {
                    'https': None, 'funnel': None,
                    'https://tailscale.com/cap/funnel-ports?ports=443,8443,10000': None}}})
        else:
            self.reply(self.server.config)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        if self.headers.get('If-Match') != str(self.server.etag):
            self.reply({}, 412)
            return
        self.server.config = json.loads(body)
        self.server.etag += 1
        self.reply({})


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


class CloudReview(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ab-review-')
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.home = root / 'home'
        self.api = Server(str(root / 'ts.sock'), LocalAPI)
        self.api.config = {}
        self.api.etag = 1
        self.addCleanup(self.api.server_close)
        self.addCleanup(self.api.shutdown)
        threading.Thread(target=self.api.serve_forever, daemon=True).start()
        self.env = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file',
                        APPLE_BRIDGE_TAILSCALE_SOCKET=str(root / 'ts.sock'),
                        APPLE_BRIDGE_REMOTE_RECHECK_MS='100')
        self.log = (root / 'gateway.log').open('wb')
        self.addCleanup(self.log.close)
        self.gateway = subprocess.Popen(
            [BINARY, 'start', '--role', 'gateway', '--home', str(self.home),
             '--api-addr', '127.0.0.1:0', '--peer-addr', '127.0.0.1:0'],
            env=self.env, stdout=self.log, stderr=self.log)
        self.addCleanup(self.stop_gateway)
        wait(lambda: (self.home / 'runtime.json').exists())
        self.enable('calendar,messages')

    def stop_gateway(self):
        self.gateway.terminate()
        self.gateway.wait(timeout=15)

    def bridge(self, *args):
        result = subprocess.run(
            [BINARY, *args, '--home', str(self.home)], env=self.env,
            capture_output=True, text=True, timeout=45)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def enable(self, sources):
        return self.bridge('remote', 'enable', '--via', 'cloud',
                           '--sources', sources, '--no-open', '--json')

    def request(self, method, path, body='', source='203.0.113.1', token='',
                content_type='application/json'):
        target = self.api.config['Web'][HOST + ':443']['Handlers']['/']['Proxy']
        target = urlsplit(target)
        headers = {'Host': HOST, 'X-Forwarded-Host': HOST,
                   'X-Forwarded-Proto': 'https', 'X-Forwarded-For': source,
                   'Content-Type': content_type}
        if method != 'GET' and path.startswith(('/auth/', '/oauth/pending')):
            headers['Origin'] = ORIGIN
        if token:
            headers['Authorization'] = 'Bearer ' + token
        connection = http.client.HTTPConnection(target.hostname, target.port, timeout=10)
        try:
            connection.request(method, target.path + path, body, headers)
            response = connection.getresponse()
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def connect(self):
        status, client = self.request('POST', '/oauth/register', json.dumps({
            'client_name': 'Review fixture', 'redirect_uris': [CALLBACK]}))
        self.assertEqual(status, 201)
        verifier = 'synthetic-review-verifier-' * 3
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
        query = urlencode({
            'response_type': 'code', 'client_id': client['client_id'],
            'redirect_uri': CALLBACK, 'scope': 'messages:read',
            'state': 'synthetic-review', 'code_challenge': challenge,
            'code_challenge_method': 'S256', 'resource': ORIGIN + '/mcp'})
        status, pending = self.request('POST', '/oauth/pending?' + query)
        self.assertEqual(status, 201)
        self.bridge('connectors', 'allow', '--request', pending['id'],
                    '--code', pending['code'], '--json')
        status, decision = self.request('GET', '/oauth/pending/' + pending['id'])
        self.assertEqual(status, 200)
        code = parse_qs(urlsplit(decision['redirect']).query)['code'][0]
        status, tokens = self.token({
            'grant_type': 'authorization_code', 'client_id': client['client_id'],
            'redirect_uri': CALLBACK, 'code': code, 'code_verifier': verifier})
        self.assertEqual(status, 200)
        return client['client_id'], tokens

    def token(self, fields):
        return self.request('POST', '/oauth/token', urlencode({
            **fields, 'resource': ORIGIN + '/mcp'}),
            content_type='application/x-www-form-urlencoded')

    def test_removed_cloud_source_stops_existing_tokens(self):
        client, tokens = self.connect()
        status, _ = self.request('POST', '/v1/op/messages.chats', '{}',
                                 token=tokens['access_token'])
        self.assertEqual(status, 200)
        self.enable('calendar')
        status, metadata = self.request(
            'GET', '/.well-known/oauth-protected-resource/mcp')
        self.assertEqual(metadata['scopes_supported'],
                         ['calendar:read', 'calendar:write'])
        status, data = self.request('POST', '/v1/op/messages.chats', '{}',
                                   token=tokens['access_token'])
        refreshed, next_tokens = self.token({
            'grant_type': 'refresh_token', 'client_id': client,
            'refresh_token': tokens['refresh_token']})
        print(json.dumps({'removed_source_read_status': status,
                          'refresh_status': refreshed,
                          'refreshed_scope': next_tokens.get('scope')}))
        self.assertIn(status, (401, 403),
                      'Messages remains readable after removing it from cloud sources')

    def test_one_funnel_source_cannot_lock_out_another(self):
        for _ in range(60):
            self.request('POST', '/auth/login/options', '{}', source='203.0.113.1')
        status, body = self.request('POST', '/auth/login/options', '{}',
                                    source='203.0.113.2')
        self.assertNotEqual(status, 429,
                            f'a different public client was locked out: {body}')


if __name__ == '__main__':
    unittest.main()
