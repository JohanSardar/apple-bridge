import http.client
import json
import pathlib
import subprocess
import sys
import tempfile
import urllib.parse

root = pathlib.Path(__file__).resolve().parents[4]
fixture = json.loads((root/'contracts/golden/oauth.json').read_text())
with tempfile.TemporaryDirectory(prefix='ab-gd1-web-golden-') as home:
    process = subprocess.Popen([sys.argv[1],home,'127.0.0.1:7780'],stdout=subprocess.PIPE,text=True)
    try:
        ready = json.loads(process.stdout.readline())
        def request(method,path,body='',form=False):
            connection = http.client.HTTPConnection('localhost',7780,timeout=10)
            connection.request(method,path,body,headers={'Origin':'http://localhost:7780','Cookie':'ab_session_local='+ready['token'],'Content-Type':'application/x-www-form-urlencoded' if form else 'application/json'})
            response = connection.getresponse()
            raw = response.read().decode()
            status = response.status
            connection.close()
            return status, raw
        passkey = json.loads((root/'contracts/golden/passkey.json').read_text())
        for flow, field in [('passkeys', 'enroll_options'), ('login', 'login_options')]:
            status, raw = request('POST', '/auth/'+flow+'/options', '{}')
            assert status == 200, raw
            options = json.loads(raw)
            ceremony_id = options.pop('ceremony_id')
            challenge = options['publicKey']['challenge']
            options['publicKey']['challenge'] = passkey[field]['publicKey']['challenge']
            assert options == passkey[field], (flow, options, passkey[field])
            expected = dict(passkey[field], ceremony_id=ceremony_id)
            expected['publicKey'] = dict(expected['publicKey'], challenge=challenge)
            assert raw == json.dumps(expected, sort_keys=True, separators=(',', ':'))+'\n', (flow, raw, expected)
        status, raw = request('POST','/oauth/register' ,json.dumps(dict(client_name='synthetic',redirect_uris=['https://connector.example/callback'])))
        assert status == 201, raw
        client = json.loads(raw)['client_id']
        replacements = {client:'client-fixture'}
        issued = None
        refresh = None
        for i, exchange in enumerate(fixture['exchanges']):
            path = exchange['path'].replace('client-fixture',client)
            body = exchange['body'].replace('client-fixture',client)
            if issued:
                body = body.replace('code-fixture',issued)
            if refresh:
                body = body.replace('refresh_token-fixture',refresh)
            status, raw = request(exchange['method'],path,body,i >= 3)
            decoded = json.loads(raw)
            if i == 2:
                issued = urllib.parse.parse_qs(urllib.parse.urlsplit(decoded['redirect']).query)['code'][0]
                replacements[issued] = 'code-fixture'
            for key in ['access_token','refresh_token']:
                if key in decoded:
                    replacements[decoded[key]] = key+'-fixture'
            refresh = decoded.get('refresh_token',refresh)
            normalized = raw
            for source,target in replacements.items():
                normalized = normalized.replace(source,target)
            assert status == exchange['status'], (i,status,raw)
            assert normalized == exchange['response'], (i,normalized,exchange['response'])
        print(json.dumps(dict(impl='rust',transcript='oauth.json',exchanges=len(fixture['exchanges']),mismatches=0)))
    finally:
        process.terminate()
        process.wait(timeout=10)
