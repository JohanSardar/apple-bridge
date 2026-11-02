import http.client
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[4]
binary = pathlib.Path(sys.argv[1]).resolve()
checks = 0
for mode in ('json', 'sse'):
    process = subprocess.Popen([str(binary), mode], stdout=subprocess.PIPE, text=True)
    try:
        address = process.stdout.readline().strip()
        host, port = address.split(':')
        def request(path, body=None, extra=None):
            headers = {'Authorization': 'Bearer synthetic', 'Host': 'localhost', 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-03-26'}
            headers.update(extra or {})
            if headers.get('Authorization') is None:
                del headers['Authorization']
            connection = http.client.HTTPConnection(host, int(port), timeout=5)
            connection.request('POST' if body is not None else 'GET', path, body, headers)
            response = connection.getresponse()
            data = response.read()
            status = response.status
            connection.close()
            return status, data
        for headers in ({'Authorization': None}, {'Host': 'evil.invalid'}, {'Origin': 'null'}, {'Origin': 'https://fixture.example.com:444'}):
            status, data = request('/v1/health', extra=headers)
            assert status == 401, (status, data)
            assert json.loads(data)['error']['code'] == 'unauthorized'
            checks += 1
        assert request('/v1/health')[0] == 200
        checks += 1
        assert request('/v1/op/records.query', b'x' * (1024 * 1024 + 1))[0] == 400
        checks += 1
        assert request('/v1/op/records.query', b'{')[0] == 400
        checks += 1
        status, data = request('/v1/op/records.query', b'{}')
        assert status == 200, data
        assert json.loads(data) == {'records': [], 'cursor': {'epoch': 'synthetic', 'subscription_revision': 1, 'sequence': 7}}
        checks += 1
        golden = json.loads((root / 'contracts/golden' / f'mcp-{mode}.json').read_text())
        for exchange in golden['exchanges'][:3]:
            status, data = request('/mcp', exchange['request'])
            assert status == exchange['status'], (status, data)
            expected = exchange['response']
            def decode(body):
                if body.startswith('event:') or body.startswith('data:'):
                    body = '\n'.join(line[5:].strip() for line in body.splitlines() if line.startswith('data:'))
                return json.loads(body) if body.strip() else None
            assert decode(data.decode()) == decode(expected), (mode, exchange['request'], data[:300])
            checks += 1
        status, data = request('/mcp', json.dumps({'jsonrpc':'2.0','id':10,'method':'tools/call','params':{'name':'records_query','arguments':{}}}))
        assert status == 200, data
        if mode == 'sse':
            data = '\n'.join(line[5:].strip() for line in data.decode().splitlines() if line.startswith('data:')).encode()
        result = json.loads(data)['result']
        assert result['structuredContent'] == json.loads(result['content'][0]['text'])
        checks += 1
    finally:
        process.terminate()
        process.wait(timeout=5)
print(json.dumps({'impl':'rust','checks':checks,'modes':['json','sse'],'golden_exchanges_per_mode':3,'golden_authorization_exchange':'unverified: outer bearer auth differs from unauthenticated direct Go handler fixture'}))
