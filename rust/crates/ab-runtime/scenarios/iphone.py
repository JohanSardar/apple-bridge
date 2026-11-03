import datetime
import ssl
import base64
import hashlib
import hmac
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
from urllib.parse import urlsplit
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ed25519, x25519
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

binary = str(Path(sys.argv[1]).resolve())
checks = []
trace = []
configuration = {'unrelated': 'preserved'}
etag = 1
lock = threading.Lock()


def raw(value):
    return json.dumps(value, separators=(',', ':')).encode()


def b64(value):
    return base64.b64encode(value).decode()


def wait(predicate):
    until = time.monotonic() + 20
    while time.monotonic() < until:
        try:
            result = predicate()
            if result:
                return result
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.05)
    raise AssertionError('fixture readiness timed out')


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args):
        pass
    def reply(self, status, value):
        trace.append((self.command,self.path,status))
        body = raw(value)
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('ETag', str(etag))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        with lock:
            if '/status' in self.path:
                self.reply(200, {'BackendState':'Running','Self':{'DNSName':'node.fixture.test.','CapMap':{'https':None,'funnel':None,'https://tailscale.com/cap/funnel-ports?ports=443,8443,10000':None}}})
            else:
                self.reply(200, configuration)
    def do_POST(self):
        global configuration, etag
        value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        with lock:
            if self.headers.get('If-Match') != str(etag):
                self.reply(412, {})
            else:
                configuration = value
                etag += 1
                self.reply(200, {})


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


with tempfile.TemporaryDirectory(prefix='ab-gf-phone-') as temp:
    home = Path(temp) / 'home'
    socket_path = str(Path(temp) / 'localapi.sock')
    server = Server(socket_path, Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    class Public(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_GET(self):
            target = urlsplit(json.loads((home / 'remote/lease.json').read_text())['target'])
            upstream = http.client.HTTPConnection(target.hostname, target.port, timeout=90)
            upstream.request('GET',target.path + self.path,headers={'Host':'node.fixture.test','X-Forwarded-Host':'node.fixture.test','X-Forwarded-Proto':'https','X-Forwarded-For':'198.51.100.4','Accept':self.headers.get('Accept','*/*')})
            response = upstream.getresponse()
            trace.append(('PUBLIC',self.path,response.status))
            self.send_response(response.status)
            for key,value in response.getheaders():
                if key.lower() not in ('transfer-encoding','connection','date','server'):
                    self.send_header(key,value)
            self.end_headers()
            try:
                while chunk := response.read1(8192):
                    self.wfile.write(chunk)
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError, ssl.SSLEOFError):
                pass
            finally:
                upstream.close()
    tls_key = ed25519.Ed25519PrivateKey.generate()
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'node.fixture.test')])
    certificate = x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(tls_key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(datetime.datetime.now(datetime.timezone.utc)-datetime.timedelta(minutes=1)).not_valid_after(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(days=1)).add_extension(x509.SubjectAlternativeName([x509.DNSName('node.fixture.test')]),critical=False).add_extension(x509.BasicConstraints(ca=True,path_length=None),critical=True).sign(tls_key,None)
    cert_path = Path(temp) / 'fixture.pem'
    key_path = Path(temp) / 'fixture.key'
    cert_path.write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(tls_key.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption()))
    public_server = http.server.ThreadingHTTPServer(('127.0.0.1',0),Public)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(cert_path,key_path)
    public_server.socket = tls.wrap_socket(public_server.socket,server_side=True)
    threading.Thread(target=public_server.serve_forever,daemon=True).start()
    env = dict(os.environ, SSL_CERT_FILE=str(cert_path), APPLE_BRIDGE_REMOTE_PROBE_ADDRESS='127.0.0.1:' + str(public_server.server_port), APPLE_BRIDGE_SECRET_STORE='file', APPLE_BRIDGE_TAILSCALE_SOCKET=socket_path, APPLE_BRIDGE_RELAY_URL='https://127.0.0.1:1')
    def command(*args):
        return subprocess.check_output([binary, *args, '--home', str(home)], env=env, text=True, timeout=100).strip()
    with (Path(temp) / 'gateway.log').open('wb') as output:
        process = subprocess.Popen([binary,'start','--role','gateway','--home',str(home),'--api-addr','127.0.0.1:0','--peer-addr','127.0.0.1:0','--blob-grace-ms','100'], env=env, stdout=output, stderr=output)
        try:
            wait(lambda: (home / 'runtime.json').exists())
            command('remote','on','--via','tailscale-app')
            def ready():
                assert process.poll() is None, (Path(temp) / 'gateway.log').read_text()
                value = json.loads((home / 'remote/status.json').read_text())
                return value if value['state'] == 'on' else None
            status = wait(ready)
            assert status['url'] == 'https://node.fixture.test'
            lease = json.loads((home / 'remote/lease.json').read_text())
            target = urlsplit(lease['target'])
            config = json.loads((home / 'runtime.json').read_text())
            checks.append('tailscale_ingress_becomes_ready')
            if '--go' not in sys.argv:
                command('remote','on','--via','relay')
                time.sleep(1.5)
                assert json.loads((home / 'remote/lease.json').read_text())['target'] == lease['target']
                command('remote','off','--via','relay')
                time.sleep(1.5)
                assert json.loads((home / 'remote/lease.json').read_text())['target'] == lease['target']
                checks.append('relay_selection_preserves_running_tailscale_adapter')
            def public(path, payload, extra=None):
                connection = http.client.HTTPConnection(target.hostname, target.port, timeout=10)
                headers = {'Host':'node.fixture.test','X-Forwarded-Host':'node.fixture.test','X-Forwarded-Proto':'https','X-Forwarded-For':'198.51.100.4','Content-Type':'application/json'}
                headers.update(extra or {})
                connection.request('POST', target.path + path, raw(payload), headers)
                response = connection.getresponse()
                code, body = response.status, response.read()
                connection.close()
                try:
                    body = json.loads(body)
                except ValueError:
                    pass
                return code, body
            token = command('token','create','--scope','iphone:read,iphone:write,system:read')
            def local(path, payload, credential=token, extra=None):
                connection = http.client.HTTPConnection(config['api_address'], timeout=10)
                headers = {'Authorization':'Bearer ' + credential,'Host':config['api_address'],'Content-Type':'application/json'}
                headers.update(extra or {})
                connection.request('POST', path, raw(payload), headers)
                response = connection.getresponse()
                code, body = response.status, response.read()
                connection.close()
                return code, json.loads(body)
            assert local('/v1/iphone/pending', {}, extra={'X-Forwarded-Proto':'https'})[0] == 401
            checks.append('untrusted_headers_cannot_mark_local_http_secure')
            code = next(value for value in command('pair','--iphone','--via-remote').split() if value.startswith('ab1-'))
            decoded = json.loads(base64.b32decode(code[4:] + '=' * (-len(code[4:]) % 8)))
            signing = ed25519.Ed25519PrivateKey.generate()
            encryption = x25519.X25519PrivateKey.generate()
            public_signing = signing.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
            public_encryption = encryption.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
            message = ('apple-bridge-iphone-pair/1\0' + decoded['id'] + '\0').encode() + public_signing + public_encryption
            pair = {'id':decoded['id'],'signing_key':b64(public_signing),'encryption_key':b64(public_encryption),'proof':b64(hmac.new(hashlib.sha256(base64.b64decode(decoded['secret'])).digest(),message,hashlib.sha256).digest()),'signature':b64(signing.sign(message))}
            result, phone = public('/v1/iphone/pair', pair)
            assert result == 200, (result, phone)
            assert public('/v1/iphone/pair', pair)[0] == 401
            checks.append('one_use_pairing_through_public_ingress')
            def phone_request(path, payload=None):
                body = raw(payload if payload is not None else {})
                at = int(time.time())
                message = ('apple-bridge-iphone/1\0' + path + '\0' + str(at) + '\0').encode() + body
                return {'device_id':phone['id'],'at':at,'payload':b64(body),'signature':b64(signing.sign(message))}
            def signed(path, payload=None):
                return public(path, phone_request(path,payload))
            assert signed('/v1/iphone/status')[1] == phone
            nil_request = phone_request('/v1/iphone/status')
            nil_request['payload'] = None
            nil_request['signature'] = b64(signing.sign(('apple-bridge-iphone/1\0/v1/iphone/status\0' + str(nil_request['at']) + '\0').encode()))
            assert public('/v1/iphone/status',nil_request)[1] == phone
            checks.append('signed_status_matches_pair_identity')
            def open_packet(packet):
                ephemeral = base64.b64decode(packet['ephemeral_key'])
                sealed = base64.b64decode(packet['box'])
                ed25519.Ed25519PublicKey.from_public_bytes(base64.b64decode(phone['gateway_key'])).verify(base64.b64decode(packet['signature']), b'apple-bridge-iphone-command/1\0' + ephemeral + sealed)
                shared = encryption.exchange(x25519.X25519PublicKey.from_public_bytes(ephemeral))
                key = HKDF(algorithm=hashes.SHA256(),length=32,salt=b'apple-bridge-iphone/1',info=phone['id'].encode()).derive(shared)
                return json.loads(AESGCM(key).decrypt(sealed[:12],sealed[12:],None))
            operation = {'command_id':'cmd_phone_fixture','args':{'device_id':phone['id']},'wait_ms':0}
            result, admitted = local('/v1/op/iphone.ping',operation)
            assert result == 200 and admitted['state'] == 'admitted', (result, admitted)
            result, packets = signed('/v1/iphone/pending')
            assert result == 200 and len(packets) == 1, (result, packets)
            job = open_packet(packets[0])
            assert job['command']['command_id'] == operation['command_id']
            assert job['request_hash'] == admitted['request_hash']
            checks.append('common_ledger_admission_and_independent_packet_decryption')
            receipt = {'command_id':operation['command_id'],'request_hash':job['request_hash'],'state':'observed','data':{'pong':True}}
            assert signed('/v1/iphone/result',dict(receipt,request_hash='wrong'))[0] != 200
            result, body = signed('/v1/iphone/result',receipt)
            assert result == 200, (result, body)
            result, observed = local('/v1/op/commands.status',{'command_id':operation['command_id']})
            assert result == 200 and observed['state'] == 'observed', (result, observed)
            assert observed['evidence']['signature']
            checks.append('signed_receipt_uses_shared_ledger_and_preserves_evidence')
            assert signed('/v1/iphone/result',receipt)[0] == 200
            assert local('/v1/op/commands.status',{'command_id':operation['command_id']})[1] == observed
            checks.append('duplicate_receipt_does_not_change_stored_status')
            for kind, content_type in [('photo','image/jpeg'),('audio','audio/wav')]:
                capture_command = {'command_id':'cmd_phone_capture_' + kind,'args':{'device_id':phone['id']},'wait_ms':0}
                result, capture = local('/v1/op/iphone.capture.' + kind,capture_command)
                assert result == 200 and capture['state'] == 'admitted', (result,capture)
                payload = ('[ab-test] phone capture ' + kind).encode()
                upload = {'command_id':capture_command['command_id'],'request_hash':capture['request_hash'],'content_type':content_type,'data':b64(payload)}
                assert signed('/v1/iphone/capture',upload)[0] == 400
                result, packets = signed('/v1/iphone/pending')
                assert result == 200 and len(packets) == 1
                assert open_packet(packets[0])['command']['command_id'] == capture_command['command_id']
                assert signed('/v1/iphone/capture',dict(upload,request_hash='wrong'))[0] == 400
                result, receipt = signed('/v1/iphone/capture',upload)
                assert result == 200 and receipt['digest'] == hashlib.sha256(payload).hexdigest() and receipt['size'] == len(payload) and receipt['content_type'] == content_type, (result,receipt)
                assert signed('/v1/iphone/capture',upload) == (200,receipt)
                assert signed('/v1/iphone/capture',dict(upload,data=b64(b'changed')))[0] == 400
                time.sleep(2.2)
                connection = http.client.HTTPConnection(config['api_address'],timeout=10)
                connection.request('GET','/v1/blobs/' + receipt['digest'] + '?record=' + receipt['blob_id'],headers={'Authorization':'Bearer ' + token,'Host':config['api_address']})
                response = connection.getresponse()
                assert response.status == 200 and response.read() == payload
                connection.close()
                result, _ = signed('/v1/iphone/result',{'command_id':capture_command['command_id'],'request_hash':capture['request_hash'],'state':'observed','data':receipt})
                assert result == 200
                other_token = command('token','create','--scope','iphone:write')
                connection = http.client.HTTPConnection(config['api_address'],timeout=10)
                connection.request('GET','/v1/blobs/' + receipt['digest'] + '?record=' + receipt['blob_id'],headers={'Authorization':'Bearer ' + other_token,'Host':config['api_address']})
                response = connection.getresponse()
                assert response.status in (401,403), response.status
                response.read()
                connection.close()
                assert local('/v1/op/commands.status',{'command_id':capture_command['command_id']})[1]['state'] == 'observed'
                checks.append('signed_' + kind + '_capture_is_bound_to_dispatch_hash_and_caller_blob_grant')
            agent = json.loads(command('agent','create','--name','synthetic-phone-approval','--level','approve','--scope','iphone:write'))
            pending_command = {'command_id':'cmd_phone_approval','args':{'device_id':phone['id']},'wait_ms':0}
            result, pending = local('/v1/op/iphone.ping',pending_command,agent['token'])
            assert result == 200 and pending['state'] == 'awaiting_approval', (result,pending)
            result, packets = signed('/v1/iphone/pending')
            assert result == 200 and len(packets) == 1
            approval = open_packet(packets[0])
            assert approval['command']['op'] == 'bridge.approval'
            decision = {'command_id':pending_command['command_id'],'request_hash':pending['request_hash'],'approve':True}
            assert signed('/v1/iphone/approval',dict(decision,request_hash='wrong'))[0] != 200
            assert local('/v1/op/commands.status',{'command_id':pending_command['command_id']},agent['token'])[1]['state'] == 'awaiting_approval'
            assert signed('/v1/iphone/approval',decision)[0] == 200
            result, packets = signed('/v1/iphone/pending')
            assert result == 200 and len(packets) == 1
            approved = open_packet(packets[0])
            assert approved['command']['command_id'] == pending_command['command_id']
            assert signed('/v1/iphone/result',{'command_id':pending_command['command_id'],'request_hash':approved['request_hash'],'state':'observed','data':{'pong':True}})[0] == 200
            checks.append('phone_approval_hash_is_checked_before_common_dispatch')
            rejected_command = dict(pending_command,command_id='cmd_phone_reject')
            result, rejected = local('/v1/op/iphone.ping',rejected_command,agent['token'])
            assert result == 200 and rejected['state'] == 'awaiting_approval'
            assert signed('/v1/iphone/approval',{'command_id':rejected_command['command_id'],'request_hash':rejected['request_hash'],'approve':False})[0] == 200
            assert local('/v1/op/commands.status',{'command_id':rejected_command['command_id']},agent['token'])[1]['state'] == 'rejected_by_owner'
            assert signed('/v1/iphone/pending')[1] == []
            checks.append('rejected_phone_approval_has_no_delivery')
            locations = [{'id':'loc_fixture','kind':'current','timestamp':810000000.0}]
            assert signed('/v1/iphone/location-records',locations)[0] == 200
            first = local('/v1/op/records.query',{'collection':'iphone.location'})[1]
            assert len(first['records']) == 1
            assert signed('/v1/iphone/location-records',locations)[0] == 200
            second = local('/v1/op/records.query',{'collection':'iphone.location'})[1]
            assert first['records'] == second['records']
            checks.append('location_capture_deduplicates_record_versions')
            command('revoke',phone['id'])
            assert signed('/v1/iphone/status')[0] == 401
            checks.append('revoked_phone_is_rejected')
            command('remote','off','--via','tailscale-app')
            wait(lambda: not (home / 'remote/lease.json').exists())
            assert configuration == {'unrelated':'preserved'}, configuration
            checks.append('remote_off_preserves_unrelated_serve_configuration')
        finally:
            if sys.exc_info()[0] is not None:
                print(json.dumps({'localapi_trace':trace}),file=sys.stderr)
            process.terminate()
            process.wait(timeout=20)
            Path(os.environ.get('AB_PROOF_LOG', '/tmp/ab-gf-phone-last-gateway.log')).write_text((Path(temp) / 'gateway.log').read_text())
            public_server.shutdown()
            public_server.server_close()
            server.shutdown()
            server.server_close()
print(json.dumps({'impl':'gateway=go' if '--go' in sys.argv else 'gateway=rust','checks':checks,'passed':True}))
