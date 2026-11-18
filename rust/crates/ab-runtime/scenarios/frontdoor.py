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
        fixture = json.loads(process.stdout.readline())
        address = fixture["address"]
        host, port = address.split(':')
        def request(path, body=None, extra=None):
            headers = {'Authorization': 'Bearer ' + fixture['token'], 'Host': 'localhost', 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-03-26'}
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
            status, data = request('/v1/openapi.json', extra=headers)
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
        assert json.loads(data)['records'] == []
        checks += 1
        golden = json.loads((root / 'contracts/golden' / f'mcp-{mode}.json').read_text())
        for exchange in golden['exchanges']:
            status, data = request('/golden-mcp', exchange['request'], {'Authorization':None})
            assert status == exchange['status'], (status, data)
            expected = exchange['response']
            def decode(body):
                if body.startswith('event:') or body.startswith('data:'):
                    body = '\n'.join(line[5:].strip() for line in body.splitlines() if line.startswith('data:'))
                return json.loads(body) if body.strip() else None
            assert decode(data.decode()) == decode(expected), (mode, exchange['request'], data[:300])
            checks += 1
        status, data = request('/mcp', json.dumps({'jsonrpc':'2.0','method':'notifications/cancelled','params':{'requestId':999}}))
        assert status == 202, (status, data)
        checks += 1
        status, data = request('/mcp', json.dumps({'jsonrpc':'2.0','id':8,'method':'initialize','params':{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'legacy-product-probe','version':'1'}}}), {'MCP-Protocol-Version':'2024-11-05'})
        assert status == 200, (status, data)
        legacy = data.decode()
        if mode == 'sse':
            legacy = '\n'.join(line[5:].strip() for line in legacy.splitlines() if line.startswith('data:'))
        assert json.loads(legacy)['result']['protocolVersion'] == '2024-11-05', legacy
        checks += 1
        def call(name, arguments, ident):
            status, data = request('/mcp', json.dumps({'jsonrpc':'2.0','id':ident,'method':'tools/call','params':{'name':name,'arguments':arguments}}))
            assert status == 200, (name, status, data)
            body = data.decode()
            if mode == 'sse':
                body = '\n'.join(line[5:].strip() for line in body.splitlines() if line.startswith('data:'))
            return json.loads(body)
        listed = request('/mcp', json.dumps({'jsonrpc':'2.0','id':11,'method':'tools/list','params':{}}))
        body = listed[1].decode()
        if mode == 'sse':
            body = '\n'.join(line[5:].strip() for line in body.splitlines() if line.startswith('data:'))
        tools = json.loads(body)['result']['tools']
        served = [tool['name'] for tool in tools]
        assert served[0] == 'apple_status' and 'records_query' not in served, served
        listed_names = {tool['name']: tool['inputSchema']['properties']['operation'].get('enum', []) for tool in tools if tool['name'] in ('apple_read', 'apple_change')}
        assert 'reminders.list' in listed_names['apple_read'] and 'reminders.items' not in listed_names['apple_read'], listed_names
        checks += 1
        reply = call('records_query', {}, 100)
        assert reply['error']['code'] == -32602 and 'apple_read' in json.dumps(reply['error']), reply
        checks += 1
        reply = call('apple_read', {'operation': 'reminders.items', 'arguments': {}}, 101)
        error = reply['result']['structuredContent']['error']
        assert reply['result']['isError'] and 'reminders.list' in json.dumps(error), error
        checks += 1
        reply = call('apple_read', {'operation': 'bridge.list_records', 'arguments': {}}, 102)
        result = reply['result']
        assert result['structuredContent'] == json.loads(result['content'][0]['text'])
        checks += 1
        reply = call('apple_describe', {'operation': 'reminders.complete'}, 400)
        contract = reply['result']['structuredContent']
        assert contract['tool'] == 'apple_change' and contract['changes_data'], contract
        checks += 1
        reply = call('apple_read', {'operation': 'reminders.list', 'arguments': {'smart': 'today', 'colour': 'red'}}, 401)
        error = reply['result']['structuredContent']['error']
        assert reply['result']['isError'] and error['field'] == 'colour' and error['example'], error
        checks += 1
        reply = call('no_such_tool', {}, 402)
        assert reply['error']['code'] == -32602 and 'apple_read' in reply['error']['data']['hint'], reply
        checks += 1
    finally:
        process.terminate()
        process.wait(timeout=5)
print(json.dumps({'impl':'rust','checks':checks,'modes':['json','sse'],'golden_exchanges_per_mode':4,'adapter':'product trust/state/transport; unauthenticated internal golden handler matches Go seam'}))
