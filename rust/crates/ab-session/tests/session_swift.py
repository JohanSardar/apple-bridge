import hashlib
import hmac
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile

BINARY, SWIFT = sys.argv[1], sys.argv[2]
SECRET = bytes(range(32))


def send(sock, value):
    body = json.dumps(value).encode()
    sock.sendall(struct.pack(">I", len(body)) + body)


def exact(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def receive(sock):
    size = struct.unpack(">I", exact(sock, 4))[0]
    return json.loads(exact(sock, size)), size + 4


def proof(role, a, b):
    return hmac.new(SECRET, b"apple-bridge-ipc/1\0" + role.encode() + b"\0" + a.encode() + b"\0" + b.encode(), hashlib.sha256).hexdigest()


home = tempfile.mkdtemp(prefix="ab-session-swift-")
os.chmod(home, 0o700)
secret = os.path.join(home, "apple-secret")
with open(secret, "wb") as handle:
    handle.write(SECRET)
os.chmod(secret, 0o600)
gateway, gateway_child = socket.socketpair()
link, link_child = socket.socketpair()
os.dup2(gateway_child.fileno(), 198, inheritable=True)
os.dup2(link.fileno(), 199, inheritable=True)
session = subprocess.Popen([BINARY, "--gateway-fd", "198", "--helper-fd", "199", "--session-secret", secret, "--session-home", home], pass_fds=(198, 199), stderr=subprocess.PIPE)
os.close(198)
os.close(199)
gateway_child.close()
link.close()
swift = subprocess.Popen([SWIFT, "--link-fd", str(link_child.fileno())], pass_fds=(link_child.fileno(),), stderr=subprocess.PIPE, stdout=subprocess.PIPE)
link_child.close()
gateway.settimeout(30)
try:
    a = "ab" * 32
    send(gateway, {"t": "Challenge", "nonce": a})
    answer, _ = receive(gateway)
    assert answer["t"] == "Authenticate" and answer["proof"] == proof("helper", a, answer["nonce"]), answer
    send(gateway, {"t": "Authenticated", "proof": proof("gateway", a, answer["nonce"])})
    send(gateway, {"t": "Open", "protocol": "ipc/1", "owner_fence": 7, "byte_credit": 4194304, "checkpoints": {"phone": {"generation": "g1", "checkpoint": "c0"}}})
    frames = []

    def until(kind):
        while True:
            frame, size = receive(gateway)
            if frame["t"] == kind:
                return frame, size
            frames.append(frame)

    ready, _ = until("Ready")
    assert ready["protocol"] == "ipc/1" and "phone" in ready["capabilities"] and "phone.set" in ready["capabilities"]["phone"], ready
    capture, size = until("Capture")
    assert capture["source"] == "phone" and capture["predecessor"] == "c0" and capture["checkpoint"].startswith("ses-"), capture
    assert [row["native_key"] for row in capture["observations"]] == ["k0", "k1", "k2"], capture
    assert capture["scan_id"] == "scan-a" and capture["scope"] == ["phone.item"] and capture["completion_evidence"] == "complete", capture
    send(gateway, {"t": "Committed", "source": "phone", "checkpoint": capture["checkpoint"], "credit": size})
    send(gateway, {"t": "Query", "id": "q1", "op": "helper.version", "args": {}, "deadline_ms": 1000})
    result, _ = until("QueryResult")
    assert result["id"] == "q1" and result["result_json"]["helper_build"] == "m1-session", result
    expires = "2099-01-01T00:00:00Z"
    action = {"op": "phone.set", "native_key": "k1", "args": {}}
    send(gateway, {"t": "Prepare", "command_id": "c1", "principal": "token:abcdef", "request_hash": "h1", "expires_at": expires, "action": action})
    prepared, _ = until("Prepared")
    assert prepared["command_id"] == "c1" and prepared["ticket"] and prepared["fingerprint"] == "h1" and prepared["native_state"]["before"] == 1, prepared
    send(gateway, {"t": "Dispatch", "command_id": "c1", "ticket": prepared["ticket"], "owner_fence": 7})
    outcome, _ = until("Outcome")
    assert outcome["command_id"] == "c1" and outcome["disposition"] == "locally_observed", outcome
    assert outcome["evidence"]["fingerprint"] == "h1" and outcome["evidence"]["prepared"]["request_hash"] == "h1", outcome
    log = open(os.path.join(home, "dispatch-guard.log")).read().splitlines()
    assert json.loads(log[0])["t"] == "DispatchStarted" and json.loads(log[-1])["t"] == "Outcome", log
    send(gateway, {"t": "Prepare", "command_id": "c1", "principal": "token:abcdef", "request_hash": "h1", "expires_at": expires, "action": action})
    refused, _ = until("Rejected")
    assert refused["error"]["code"] == "conflict", refused
finally:
    print("exit codes", session.poll(), swift.poll(), file=sys.stderr)
    swift.kill()
    session.kill()
    swift.wait()
    session.wait()
    for name, process in (("session", session), ("swift", swift)):
        text = process.stderr.read().decode() if process.stderr else ""
        if text:
            print(name, "stderr:", text[-600:], file=sys.stderr)
print("ok swift helper against ab-session")
