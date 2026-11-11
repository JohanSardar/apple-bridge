import socket
import sqlite3
import fcntl
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

    def connect(self, exchanged=200):
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
        self.assertEqual(status, exchanged)
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


    def test_source_change_revokes_refresh_and_access(self):
        client, tokens = self.connect()
        self.enable("calendar")
        read, _ = self.request("POST", "/v1/op/messages.chats", "{}",
                               token=tokens["access_token"])
        refreshed, _ = self.token({
            "grant_type": "refresh_token", "client_id": client,
            "refresh_token": tokens["refresh_token"]})
        print(json.dumps({"normal_read": read, "normal_refresh": refreshed}))
        self.assertIn(read, (401, 403))
        self.assertEqual(refreshed, 400)

    def test_interrupted_narrowing_retries_revocation(self):
        client, tokens = self.connect()
        with (self.home / "credentials.lock").open("r+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            child = subprocess.Popen([
                BINARY, "remote", "enable", "--via", "cloud",
                "--sources", "calendar", "--no-open", "--json",
                "--home", str(self.home)], env=self.env,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                time.sleep(2)
                self.assertIsNone(child.poll())
                self.assertNotEqual(json.loads(
                    (self.home / "remote/config.json").read_text()
                ).get("cloud_sources"), ["calendar"],
                    "the narrower list was saved before old connections were revoked")
            finally:
                child.kill()
                child.wait(timeout=10)
                fcntl.flock(lock, fcntl.LOCK_UN)
        self.enable("calendar")
        status, metadata = self.request(
            "GET", "/.well-known/oauth-protected-resource/mcp")
        self.assertEqual(status, 200)
        self.assertEqual(metadata["scopes_supported"],
                         ["calendar:read", "calendar:write"])
        read, _ = self.request("POST", "/v1/op/messages.chats", "{}",
                               token=tokens["access_token"])
        refreshed, _ = self.token({
            "grant_type": "refresh_token", "client_id": client,
            "refresh_token": tokens["refresh_token"]})
        print(json.dumps({"interrupted_then_retried_read": read,
                          "interrupted_then_retried_refresh": refreshed}))
        self.assertIn(read, (401, 403),
                      "a successful retry left the removed source readable")
        self.assertEqual(refreshed, 400)

    def test_prechange_pending_request_cannot_restore_removed_source(self):
        original = self.bridge

        def narrow_then_allow(*args):
            if args[:2] == ("connectors", "allow"):
                self.enable("calendar")
            return original(*args)

        self.bridge = narrow_then_allow
        _, refused = self.connect(exchanged=400)
        status, metadata = self.request(
            "GET", "/.well-known/oauth-protected-resource/mcp")
        self.assertEqual(status, 200)
        self.assertEqual(metadata["scopes_supported"],
                         ["calendar:read", "calendar:write"])
        print(json.dumps({"prechange_pending_exchange": refused}))
        self.assertNotIn("access_token", refused,
                         "an old pending request restored a removed cloud source")

    def assert_refused(self, client, tokens):
        read, _ = self.request("POST", "/v1/op/messages.chats", "{}",
                               token=tokens["access_token"])
        refreshed, refused = self.token({
            "grant_type": "refresh_token", "client_id": client,
            "refresh_token": tokens["refresh_token"]})
        self.assertEqual(read, 401)
        self.assertEqual(refreshed, 400)
        self.assertEqual(refused, {"error": "invalid_grant"})
        self.assertNotIn("access_token", refused)
        self.assertNotIn("refresh_token", refused)
        print(json.dumps({"new_request": read, "refresh": refreshed}))
    def append_synthetic_chat_change(self, marker):
        with sqlite3.connect(self.home / "state.db", timeout=10) as db:
            cursor = json.loads(db.execute(
                "SELECT value FROM meta WHERE key='cursor'").fetchone()[0])
            cursor["sequence"] += 1
            sequence = cursor["sequence"]
            record = {
                "collection": "messages.chat",
                "id": "cht_" + f"{sequence:026d}",
                "version": 1, "source": "messages",
                "observed_at": "2026-11-11T00:00:00Z",
                "deleted": False,
                "coverage": {"fields": "full", "missing": None},
                "data": {"display_name": marker},
            }
            change = {"sequence": sequence, "op": "upsert", "record": record}
            db.execute(
                "INSERT INTO journal(seq,epoch,source,payload) VALUES(?,?,?,?)",
                (sequence, cursor["epoch"], "messages",
                 json.dumps(change).encode()))
            db.execute("INSERT INTO journal_commits VALUES(?,?)",
                       (cursor["epoch"], sequence))
            db.execute("UPDATE meta SET value=? WHERE key='cursor'",
                       (json.dumps(cursor),))
    def open_stream(self, token):
        target = urlsplit(
            self.api.config["Web"][HOST + ":443"]["Handlers"]["/"]["Proxy"])
        connection = http.client.HTTPConnection(
            target.hostname, target.port, timeout=3)
        self.addCleanup(connection.close)
        connection.request("GET", target.path + "/v1/changes", headers={
            "Host": HOST, "X-Forwarded-Host": HOST,
            "X-Forwarded-Proto": "https", "X-Forwarded-For": "203.0.113.1",
            "Authorization": "Bearer " + token})
        response = connection.getresponse()
        self.assertEqual(response.status, 200)
        self.assertEqual(response.readline(), b": connected\n")
        self.assertEqual(response.readline(), b"\n")
        return response
    def next_data(self, response):
        try:
            while True:
                line = response.readline()
                if not line:
                    return None
                if line.startswith(b"data: "):
                    return json.loads(line[6:])
        except (TimeoutError, socket.timeout):
            return None
    def test_interrupted_narrowing_hold_only(self):
        with (self.home / "credentials.lock").open("r+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            child = subprocess.Popen([
                BINARY, "remote", "enable", "--via", "cloud",
                "--sources", "calendar", "--no-open", "--json",
                "--home", str(self.home)], env=self.env,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                time.sleep(2)
                self.assertIsNone(child.poll())
                self.assertNotEqual(json.loads(
                    (self.home / "remote/config.json").read_text()
                ).get("cloud_sources"), ["calendar"])
            finally:
                child.kill()
                child.wait(timeout=10)
                fcntl.flock(lock, fcntl.LOCK_UN)
    def test_existing_stream_after_interrupted_narrowing(self):
        client, tokens = self.connect()
        response = self.open_stream(tokens["access_token"])
        self.append_synthetic_chat_change("AB_REVIEW_R3_BEFORE")
        before = self.next_data(response)
        self.assertIn("AB_REVIEW_R3_BEFORE", json.dumps(before))
        self.test_interrupted_narrowing_hold_only()
        self.enable("calendar")
        self.assert_refused(client, tokens)
        self.append_synthetic_chat_change("AB_REVIEW_R3_AFTER_NARROWING")
        after = self.next_data(response)
        print(json.dumps({"post_narrowing_stream_batch": after}))
        self.assertNotIn(
            "AB_REVIEW_R3_AFTER_NARROWING", json.dumps(after),
            "an already-open cloud stream exposed a removed source after narrowing")
    def test_existing_stream_after_normal_narrowing(self):
        client, tokens = self.connect()
        response = self.open_stream(tokens["access_token"])
        self.append_synthetic_chat_change("AB_REVIEW_R3_BEFORE")
        self.assertIn("AB_REVIEW_R3_BEFORE", json.dumps(self.next_data(response)))
        self.enable("calendar")
        self.assert_refused(client, tokens)
        self.append_synthetic_chat_change("AB_REVIEW_R3_AFTER_NORMAL_NARROWING")
        after = self.next_data(response)
        print(json.dumps({"normal_post_narrowing_stream_batch": after}))
        self.assertNotIn("AB_REVIEW_R3_AFTER_NORMAL_NARROWING", json.dumps(after))


if __name__ == '__main__':
    unittest.main()
