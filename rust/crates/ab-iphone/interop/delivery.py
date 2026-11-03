import json
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error

fixture, rust, go = sys.argv[1:]
checks = 0


def invoke(binary, **request):
    out = subprocess.run([binary], input=json.dumps(request), text=True, capture_output=True, check=True)
    return json.loads(out.stdout)


def check(ok):
    global checks
    checks += 1
    assert ok, f"assertion {checks} failed"


with tempfile.TemporaryDirectory(prefix="ab-gd3-delivery-") as home:
    code = invoke(go, home=home, op="code")["code"]
    pair = invoke(go, op="pair-request", code=code)
    device = invoke(rust, op="pair", home=home, request=pair)
    device_id = device["id"]
    now = int(time.time())
    store = {"rows": []}

    def route(path, payload=None, at=None):
        global store
        request = invoke(go, op="request", id=device_id, path=path, payload=payload or {}, at=at or now)
        result = invoke(fixture, op="delivery", home=home, path=path, request=request, now=now, store=store)
        store = result["store"]
        return result["result"]

    batch = {"stream": "steps", "sequence": 1, "samples": [{"id": "s1", "type": "steps", "start": "2026-11-01T01:00:00Z", "end": "2026-11-01T02:00:00Z", "value": 2.0}], "deleted": []}
    check(invoke(fixture, op="health-digest", home=home, batch=batch) == invoke(go, op="health-digest", batch=batch))
    for value in (0.0, -0.0, 1e-7, 1e-6, 1e20, 1e21, 1.23456789):
        sample = dict(batch["samples"][0], value=value, details={"v": value, "nested": [value]})
        candidate = dict(batch, samples=[sample])
        check(invoke(fixture, op="health-digest", home=home, batch=candidate) == invoke(go, op="health-digest", batch=candidate))
    check(route("/v1/iphone/health", batch) == {"accepted": True})
    check(route("/v1/iphone/health", batch) == {"accepted": True})
    check("error" in route("/v1/iphone/health", dict(batch, sequence=3)))
    changed = dict(batch, samples=[])
    check("error" in route("/v1/iphone/health", changed))
    check(route("/v1/iphone/health", dict(batch, stream="sleep", samples=[])) == {"accepted": True})
    check("error" in route("/v1/iphone/health", batch, now - 120))
    for bad in (dict(batch, sequence=0), dict(batch, samples=[dict(batch["samples"][0], type="sleep")]), dict(batch, deleted=[""])):
        check("error" in route("/v1/iphone/health", bad))
    locations = [{"id":"loc1", "kind":"current", "timestamp": 810000000}]
    check(route("/v1/iphone/location-records", locations) == {"accepted": True})
    check(route("/v1/iphone/location-records", locations) == {"accepted": True})
    check(len(store["locations"]) == 1)
    check("error" in route("/v1/iphone/location-records", [dict(locations[0],kind="unknown")]))
    check(route("/v1/iphone/push-token", {"token":"a"*64}) == {"registered": True})
    check(route("/v1/iphone/live-token", {"task_id":"_start","token":"b"*64}) == {"registered": True})
    command = {"command_id":"cmd_fixture", "op":"iphone.live.start", "args":{"device_id":device_id,"task_id":"task","title":"Synthetic <task>","status":"working","progress":0.25}}
    check(invoke(fixture,op="live",home=home,id=device_id,command=command,now=now) == invoke(go,op="live",home=home,id=device_id,command=command,now=now))
    check(route("/v1/iphone/live-disable") == {"disabled": True})
    check("error" in invoke(fixture,op="live",home=home,id=device_id,command=command,now=now))
    command = dict(command, op="iphone.ping",args={"device_id":device_id},expires_at="0001-01-01T00:00:00Z")
    row = {"command":command,"principal":"token:fixture","source":"iphone","request_hash":"fixture-hash","state":"admitted","created_at":"2026-11-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z"}
    store["rows"] = [row]
    packets = route("/v1/iphone/pending")
    check(len(packets) == 1)
    check(store["rows"][0]["state"] == "dispatched")
    opened = invoke(go,op="open",id=device_id,packet=packets[0],gateway_key=device["gateway_key"])
    job = json.loads(opened["plaintext"])
    check(job["command"] == command and job["request_hash"] == "fixture-hash")
    receipt = {"command_id":"cmd_fixture","request_hash":"wrong","state":"observed","data":{"pong":True}}
    check("error" in route("/v1/iphone/result",receipt))
    receipt["request_hash"] = "fixture-hash"
    check(route("/v1/iphone/result",receipt) == {"accepted":True})
    check(store["rows"][0]["state"] == "observed")
    check(store["evidence"][0]["device_id"] == device_id)
    check(route("/v1/iphone/result",receipt) == {"accepted":True} and len(store["evidence"]) == 1)
    for prior, expected in (("admitted","expired"),("prepared","expired"),("dispatched","indeterminate")):
        store["rows"] = [dict(row,state=prior,expires_at="2020-01-01T00:00:00Z")]
        check(route("/v1/iphone/pending") == [])
        check(store["rows"][0]["state"] == expected)
    store["rows"] = [dict(row,state="dispatched")]
    server = subprocess.Popen([fixture], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        server.stdin.write(json.dumps({"op":"serve", "home":home,"store":store}))
        server.stdin.close()
        address = server.stdout.readline().strip()
        check(address.startswith("http://127.0.0.1:"))
        path = "/v1/iphone/pending"
        signed = invoke(go, op="request", id=device_id, path=path)
        request = urllib.request.Request(address+path, data=json.dumps(signed).encode(), headers={"Content-Type":"application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:
            body = response.read()
            check(response.status == 200 and body.endswith(b"\n"))
            check(body.startswith(b'[{"ephemeral_key":'))
        bad = dict(signed, signature="AA==")
        try:
            urllib.request.urlopen(urllib.request.Request(address+path,data=json.dumps(bad).encode()),timeout=5)
            check(False)
        except urllib.error.HTTPError as error:
            check(error.code == 401)
    finally:
        server.terminate()
        server.wait(timeout=5)
    check(invoke(fixture,op="list",home=home) == invoke(go,op="list",home=home))
    check(invoke(fixture,op="revoke",home=home,id=device_id) == {"revoked":True})
    check(invoke(go,op="list",home=home) == [])
    check("error" in route("/v1/iphone/pending"))
    result = invoke(fixture,op="delivery",home=home,path="sweep",now=now,store=store)
    check(result["store"]["rows"][0]["state"] == "indeterminate")
print(json.dumps({"impl":"rust","oracle":"go","checks":checks,"failures":0,"qualification":"crate_fixture_only"}))
