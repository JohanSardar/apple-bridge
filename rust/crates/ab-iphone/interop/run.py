import concurrent.futures
import json
import subprocess
import sys
import tempfile
import time

rust, go = sys.argv[1:]
checks = 0


def invoke(binary, **request):
    result = subprocess.run([binary], input=json.dumps(request), text=True, capture_output=True, check=True)
    return json.loads(result.stdout)


def check(condition):
    global checks
    assert condition, f"assertion {checks + 1} failed"
    checks += 1


with tempfile.TemporaryDirectory(prefix="ab-gd-iphone-") as home:
    for maker, consumer in ((rust, rust), (go, rust), (rust, go), (go, go)):
        code = invoke(maker, op="code", home=home)["code"]
        pair = invoke(go, op="pair-request", code=code)
        bad_pair = dict(pair, proof="AA==")
        check("error" in invoke(consumer, op="pair", home=home, request=bad_pair, pair=bad_pair))
        bad_pair = dict(pair, signature="AA==")
        check("error" in invoke(consumer, op="pair", home=home, request=bad_pair, pair=bad_pair))
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda _: invoke(consumer, op="pair", home=home, request=pair, pair=pair), range(2)))
        check(sum("id" in result for result in results) == 1)
        check(sum("error" in result for result in results) == 1)
        device = next(result for result in results if "id" in result)
        device_id = device["id"]
        principal = invoke(go, op="principal", home=home, id=device_id)
        check(principal["kind"] == "iphone")
        path = "/v1/iphone/result"
        request = invoke(go, op="request", id=device_id, path=path)
        check(invoke(rust, op="verify", home=home, path=path, request=request)["id"] == device_id)
        check(invoke(go, op="verify", home=home, path=path, request=request)["id"] == device_id)
        for binary in (rust, go):
            check("error" in invoke(binary, op="verify", home=home, path="/v1/iphone/approval", request=request))
            stale = invoke(go, op="request", id=device_id, path=path, at=int(time.time()) - 120)
            check("error" in invoke(binary, op="verify", home=home, path=path, request=stale))
            future = invoke(go, op="request", id=device_id, path=path, at=int(time.time()) + 120)
            check("error" in invoke(binary, op="verify", home=home, path=path, request=future))
        packet = invoke(rust, op="seal", home=home, id=device_id)
        check(invoke(rust, op="push-json", home=home, packet=packet["packet"])["raw"] == invoke(go, op="push-json", packet=packet["packet"])["raw"])
        check(invoke(go, op="open", id=device_id, **packet)["plaintext"] == "synthetic-command")
        check("error" in invoke(go, op="open", id="wrong-device", **packet))
        tampered = dict(packet, packet=dict(packet["packet"], signature="AA=="))
        check("error" in invoke(go, op="open", id=device_id, **tampered))
        invoke(rust, op="tokens", home=home, id=device_id)
        principal = invoke(go, op="principal", home=home, id=device_id)
        check(principal["iphone_push_token"] == "a" * 64)
        check(principal["iphone_live_tokens"]["_start"] == "b" * 64)
print(json.dumps({"impl":"rust", "oracle":"go", "checks":checks,"failures":0}))
