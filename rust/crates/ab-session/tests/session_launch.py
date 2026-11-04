import hashlib
import hmac
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile

BINARY = sys.argv[1]
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
    return json.loads(exact(sock, size))


def proof(role, a, b):
    message = b"apple-bridge-ipc/1\0" + role.encode() + b"\0" + a.encode() + b"\0" + b.encode()
    return hmac.new(SECRET, message, hashlib.sha256).hexdigest()


def prepare_home(mode=0o600):
    home = tempfile.mkdtemp(prefix="ab-session-launch-")
    os.chmod(home, 0o700)
    secret = os.path.join(home, "apple-secret")
    with open(secret, "wb") as handle:
        handle.write(SECRET)
    os.chmod(secret, mode)
    return home, secret


def launch(arguments):
    pairs = {name: socket.socketpair() for name in ("gateway", "helper")}
    targets = {"gateway": 198, "helper": 199}
    for name, (_, child) in pairs.items():
        os.dup2(child.fileno(), targets[name], inheritable=True)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("AB_")}
    process = subprocess.Popen([BINARY, "--gateway-fd", "198", "--helper-fd", "199"] + arguments, env=environment, pass_fds=(198, 199), stderr=subprocess.PIPE)
    for _, child in pairs.values():
        child.close()
    for descriptor in targets.values():
        os.close(descriptor)
    sockets = {name: pair[0] for name, pair in pairs.items()}
    for sock in sockets.values():
        sock.settimeout(15)
    return process, sockets


def finish(process, sockets):
    for sock in sockets.values():
        sock.close()
    process.kill()
    process.wait()


home, secret = prepare_home()
process, sockets = launch(["--session-secret", secret, "--session-home", home])
try:
    gateway, helper = sockets["gateway"], sockets["helper"]
    a = "ab" * 32
    send(gateway, {"t": "Challenge", "nonce": a})
    answer = receive(gateway)
    assert answer["t"] == "Authenticate", answer
    assert answer["proof"] == proof("helper", a, answer["nonce"]), answer
    send(gateway, {"t": "Authenticated", "proof": proof("gateway", a, answer["nonce"])})
    send(gateway, {"t": "Open", "protocol": "ipc/1", "owner_fence": 7, "byte_credit": 4194304, "checkpoints": {"reminders": {"generation": "g", "checkpoint": "c0"}}})
    hello = receive(helper)
    assert hello == {"t": "Hello", "protocol": "native/1", "version": 1, "owner_fence": 7}, hello
    send(helper, {"t": "Welcome", "protocol": "native/1", "version": 1})
    session = receive(helper)
    assert session["t"] == "Session" and session["checkpoints"]["reminders"]["checkpoint"] == "c0", session
    send(helper, {"t": "Ready", "coverage": {"reminders": {"state": "ready"}}, "capabilities": {"reminders": []}, "helper_build": "t", "os_build": "o"})
    ready = receive(gateway)
    assert ready["t"] == "Ready" and ready["protocol"] == "ipc/1", ready
    send(helper, {"t": "Scan", "id": "s1", "source": "reminders", "generation": "g", "observations": [{"kind": "upsert", "collection": "reminders.reminder", "native_key": "k", "data": {"title": "x"}, "observed_at": "2026-11-03T00:00:00Z"}]})
    capture = receive(gateway)
    assert capture["t"] == "Capture" and capture["predecessor"] == "c0", capture
    assert capture["checkpoint"].startswith("ses-"), capture
    body = json.dumps(capture).encode()
    send(gateway, {"t": "Committed", "source": "reminders", "checkpoint": capture["checkpoint"], "credit": 4 + len(json.dumps(capture, separators=(",", ":")).encode())})
    committed = receive(helper)
    assert committed["t"] == "ScanCommitted" and committed["checkpoint"] == capture["checkpoint"], committed
finally:
    finish(process, sockets)
print("ok session launch")

home, secret = prepare_home()
process, sockets = launch(["--session-secret", secret, "--session-home", home])
try:
    send(sockets["gateway"], {"t": "Challenge", "nonce": "cd" * 32})
    answer = receive(sockets["gateway"])
    send(sockets["gateway"], {"t": "Authenticated", "proof": proof("helper", "cd" * 32, answer["nonce"])})
    assert process.wait(timeout=15) != 0
    try:
        receive(sockets["helper"])
        raise AssertionError("helper heard from a session that failed authentication")
    except (EOFError, ConnectionError):
        pass
finally:
    finish(process, sockets)
print("ok wrong gateway proof refused")

home, secret = prepare_home(0o644)
process, sockets = launch(["--session-secret", secret, "--session-home", home])
assert process.wait(timeout=15) != 0
finish(process, sockets)
print("ok loose secret refused")

home, secret = prepare_home()
process, sockets = launch(["--session-secret", secret])
assert process.wait(timeout=15) != 0
finish(process, sockets)
print("ok missing home refused")

home, secret = prepare_home()
process, sockets = launch(["--session-secret", secret, "--session-home", home, "--capture-home", home])
assert process.wait(timeout=15) != 0
finish(process, sockets)
print("ok an unknown flag is refused")
