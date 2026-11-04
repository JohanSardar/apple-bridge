import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading

BINARY = sys.argv[1]


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


def row(collection, key, title):
    return {"kind": "upsert", "collection": collection, "native_key": key, "scoped_alias": "a/" + key, "data": {"title": title}, "observed_at": "2026-11-03T00:00:00Z"}


def serve_facts(sock, seen):
    counter = 0
    try:
        while True:
            frame = receive(sock)
            seen.append(frame["t"])
            if frame["t"] == "Hello":
                send(sock, {"t": "Welcome", "protocol": "native/1", "version": 1})
                continue
            counter += 1
            items = [row("calendar.event", "event-1", "One")] if frame["op"] == "agenda.events" else [row("calendar.calendar", "cal-1", "Home")]
            send(sock, {"t": "FactPage", "id": frame["id"], "op": frame["op"], "snapshot": "s%d" % counter, "items": items, "cursor": None, "complete": True, "scope": {"collections": ["x"], "history_token": ""}, "generation": "eventkit-calendar-v1"})
    except (EOFError, OSError):
        return


def launch(effects, facts_variable, facts_descriptor, effects_variable=True):
    home = tempfile.mkdtemp(prefix="ab-fd-launch-")
    os.makedirs(os.path.join(home, "session"))
    with open(os.path.join(home, "session", "rust-capture.json"), "w") as handle:
        json.dump({"sources": {"calendar": {"root": home, "facts": "native", "facts_poll_ms": 50}}}, handle)
    pairs = {name: socket.socketpair() for name in ("gateway", "helper", "native", "facts")}
    targets = {"gateway": 198, "helper": 199, "native": 200, "facts": 201}

    for name, (_, child) in pairs.items():
        os.dup2(child.fileno(), targets[name], inheritable=True)

    environment = {key: value for key, value in os.environ.items() if not key.startswith("AB_")}
    if effects_variable:
        environment["AB_NATIVE_FD"] = "200"
    if facts_variable:
        environment["AB_NATIVE_FACTS_FD"] = str(facts_descriptor)
    arguments = [BINARY, "--gateway-fd", "198", "--helper-fd", "199", "--capture-home", home]
    if effects:
        environment["AB_SESSION_EFFECTS"] = "1"
        arguments += ["--effects-home", home]
    process = subprocess.Popen(arguments, env=environment, pass_fds=(198, 199, 200, 201), stderr=subprocess.PIPE)
    for _, child in pairs.values():
        child.close()
    for descriptor in targets.values():
        os.close(descriptor)
    return process, {name: pair[0] for name, pair in pairs.items()}


def converse(process, sockets, effects_expected):
    seen_facts = []
    threading.Thread(target=serve_facts, args=(sockets["facts" if effects_expected else "native"], seen_facts), daemon=True).start()
    for sock in sockets.values():
        sock.settimeout(15)
    send(sockets["gateway"], {"t": "Open", "protocol": "ipc/1", "owner_fence": 7, "byte_credit": 4194304, "checkpoints": {}})
    if effects_expected:
        hello = receive(sockets["native"])
        assert hello["t"] == "Hello" and hello["owner_fence"] == 7, hello
        send(sockets["native"], {"t": "Welcome", "protocol": "native/1", "version": 1})
    assert receive(sockets["helper"])["t"] == "Open"
    send(sockets["helper"], {"t": "Ready", "protocol": "ipc/1"})
    while True:
        frame = receive(sockets["gateway"])
        if frame["t"] == "Capture" and frame["source"] == "calendar":
            break
    keys = [observation["native_key"] for observation in frame["observations"]]
    assert keys == ["cal-1", "event-1"], keys
    assert seen_facts[0] == "Hello" and seen_facts.count("Facts") >= 2, seen_facts


def run_case(name, effects, facts_variable, facts_descriptor, effects_expected):
    process, sockets = launch(effects, facts_variable, facts_descriptor)
    try:
        converse(process, sockets, effects_expected)
    finally:
        process.kill()
        process.wait()
    print("ok", name)


run_case("effects and facts on separate streams", True, True, 201, True)
run_case("facts only falls back to the native stream", False, False, 0, False)
run_case("facts only on the facts stream", False, True, 201, False)

process, sockets = launch(True, True, 200)
code = process.wait(timeout=15)
assert code != 0, code
print("ok shared descriptor refused")
process, sockets = launch(True, False, 0)
code = process.wait(timeout=15)
assert code != 0, code
print("ok effects without a facts descriptor refused")
