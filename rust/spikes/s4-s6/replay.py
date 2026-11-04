import http.client
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[3]
BINARY = ROOT / "rust/spikes/s4-s6/target/debug/ab-spike-s4-s6"

def payload(body, content_type):
    if "text/event-stream" in content_type:
        events = [line[5:].strip() for line in body.splitlines() if line.startswith("data:")]
        return json.loads(events[-1]) if events else None
    return json.loads(body) if body.strip() else None

def differences(left, right, path=""):
    if isinstance(left, dict) and isinstance(right, dict):
        return [item for key in sorted(left.keys() | right.keys()) for item in differences(left.get(key), right.get(key), path + "/" + key)]
    if left != right:
        return [{"path": path, "golden": left, "rust": right}]
    return []

def run():
    report = {}
    for mode in ("json", "sse"):
        server = subprocess.Popen([str(BINARY), "serve", mode], cwd=ROOT, stdout=subprocess.PIPE, text=True)
        try:
            address = server.stdout.readline().strip()
            host, port = address.split(":")
            fixture = json.loads((ROOT / ("contracts/golden/mcp-" + mode + ".json")).read_text())
            def request(body, headers=None):
                conn = http.client.HTTPConnection(host, int(port), timeout=5)
                conn.request("POST", "/mcp", body, {"Content-Type":"application/json", "Accept":"application/json, text/event-stream", "MCP-Protocol-Version":"2025-03-26", **(headers or {})})
                response = conn.getresponse()
                result = response.status, response.getheader("Content-Type", ""), response.read().decode()
                conn.close()
                return result
            results = []
            for exchange in fixture["exchanges"]:
                status, ct, body = request(exchange["request"])
                results.append({"method":json.loads(exchange["request"])["method"], "status":status, "content_type":ct, "differences":differences(payload(exchange["response"], exchange["content_type"]), payload(body, ct)), "status_matches":status == exchange["status"], "wire_matches":body == exchange["response"]})
            assert all(not r["differences"] and r["status_matches"] for r in results), results
            security = {}
            for label, headers in {"bad_host":{"Host":"evil.example"}, "bad_origin":{"Origin":"https://evil.example"}, "allowed_origin":{"Origin":"http://localhost:7780"}, "public_origin":{"Host":"fixture.example.com", "Origin":"https://fixture.example.com"}, "wrong_port":{"Origin":"http://localhost:7781"}, "null_origin":{"Origin":"null"}}.items():
                security[label] = request(fixture["exchanges"][0]["request"], headers)[0]
            assert security == {"bad_host":403, "bad_origin":403, "allowed_origin":200, "public_origin":200, "wrong_port":403, "null_origin":403}, security
            versions = {}
            for version in ("2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"):
                body = json.loads(fixture["exchanges"][0]["request"])
                body["params"]["protocolVersion"] = version
                status, ct, body = request(json.dumps(body), {"MCP-Protocol-Version":version})
                versions[version] = {"status":status, "body":payload(body,ct)}
            assert all(v["status"] == 200 and v["body"]["result"]["protocolVersion"] == key for key,v in versions.items()), versions
            client = subprocess.run([str(BINARY), "client", "http://" + address + "/mcp"], cwd=ROOT, capture_output=True, text=True, timeout=20)
            assert client.returncode == 0, client.stderr
            report[mode] = {"exchanges":results, "security":security, "versions":versions, "rmcp_client":{"status":client.returncode,"stdout":client.stdout,"stderr":client.stderr}}
        finally:
            server.terminate()
            server.wait()
    print(json.dumps(report, indent=2))

if __name__ == "__main__":
    run()
