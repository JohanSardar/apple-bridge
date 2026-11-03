import base64
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile

rust, go = sys.argv[1:3]
root = pathlib.Path(__file__).resolve().parents[4]
checks = 0
os.environ["APPLE_BRIDGE_SECRET_STORE"] = "file"

def invoke(binary, home, op, **data):
    return json.loads(subprocess.check_output([binary], input=json.dumps(dict(home=str(home), op=op, **data)).encode()))

def check(ok, label):
    global checks
    if not ok:
        raise AssertionError(label)
    checks += 1

with tempfile.TemporaryDirectory(prefix='ab-gd1-oauth-') as temporary:
    for issuer in [rust, go]:
        for exchanger in [rust, go]:
            home = pathlib.Path(temporary) / str(checks)
            c = invoke(issuer, home, 'register', client=dict(client_name='synthetic', redirect_uris=['https://connector.example/callback']))
            check('client_id' in c, 'registration')
            verifier = 'a' * 43
            code = dict(hash='', client_id=c['client_id'], redirect_uri=c['redirect_uris'][0], challenge=base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('='), resource='http://localhost:7780/mcp', principal='', expires_at='0001-01-01T00:00:00Z', used=False)
            issued = invoke(issuer, home, 'issue', code=code)
            check('code' in issued, 'issue')
            form = dict(grant_type='authorization_code', client_id=c['client_id'], code=issued['code'], redirect_uri=code['redirect_uri'], code_verifier=verifier, resource=code['resource'])
            check('error' in invoke(exchanger, home, 'exchange', exchange=dict(form, code_verifier='b'*43)), 'wrong PKCE rejected')
            tokens = invoke(exchanger, home, 'exchange', exchange=form)
            check(tokens.get('expires_in') == 3600, 'exchange lifetime')
            for reader in [rust, go]:
                p = invoke(reader, home, 'authenticate', token=tokens['access_token'])
                check(p.get('id') == issued['principal'] and p.get('level') == 'approve' and p.get('grants') == ['messages:write'], 'cross-language access')
            refresh = dict(grant_type='refresh_token', client_id=c['client_id'], resource=code['resource'], refresh_token=tokens['refresh_token'])
            rotated = invoke(rust, home, 'exchange', exchange=refresh)
            check(rotated.get('refresh_token') != tokens['refresh_token'] and 'error' not in rotated, 'refresh rotation')
            data = json.loads((home/'credentials.json').read_text())
            check(data['oauth_refresh'][0]['expires_at'] == data['oauth_refresh'][1]['expires_at'], 'absolute refresh expiry')
            stored = (home/'credentials.json').read_text()
            check(all(s not in stored for s in [tokens['access_token'], tokens['refresh_token'], issued['code']]), 'hashed secrets')
            check('error' in invoke(rust, home, 'exchange', exchange=refresh), 'reuse detected')
            for reader in [rust, go]:
                check('error' in invoke(reader, home, 'authenticate', token=rotated['access_token']), 'reuse persisted family revocation')
    home = pathlib.Path(temporary) / 'passkey'
    fixture = json.loads((root/'contracts/golden/passkey.json').read_text())
    result = invoke(rust, home, 'golden', fixture=fixture)
    check(result == dict(accepted=True, counter=2), 'Go golden passkey signs in under Rust')
print(json.dumps(dict(assertions=checks, failures=0, impl='rust', scope='OAuth durable interop and Go passkey golden authentication')))
