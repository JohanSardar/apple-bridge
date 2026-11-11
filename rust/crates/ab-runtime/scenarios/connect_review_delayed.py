import fcntl
import http.client
import importlib.util
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import unittest
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "connect_review", HERE / "connect_review.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class DelayedBodyReview(fixture.CloudReview):
    def begin_delayed_post(self, path, body, token):
        target = urlsplit(
            self.api.config["Web"][fixture.HOST + ":443"]["Handlers"]["/"]["Proxy"])
        sock = socket.create_connection((target.hostname, target.port), timeout=10)
        self.addCleanup(sock.close)
        headers = {
            "Host": fixture.HOST,
            "X-Forwarded-Host": fixture.HOST,
            "X-Forwarded-Proto": "https",
            "X-Forwarded-For": "203.0.113.1",
            "Authorization": "Bearer " + token,
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": "2025-03-26",
            "Content-Length": str(len(body)),
            "Expect": "100-continue",
            "Connection": "close",
        }
        request = "POST " + target.path + path + " HTTP/1.1\r\n"
        request += "".join(k + ": " + v + "\r\n" for k, v in headers.items())
        sock.sendall((request + "\r\n").encode())
        interim = bytearray()
        while not interim.endswith(b"\r\n\r\n"):
            byte = sock.recv(1)
            self.assertTrue(byte, "connection closed before request-body polling")
            interim.extend(byte)
        self.assertEqual(bytes(interim), b"HTTP/1.1 100 Continue\r\n\r\n")
        print(json.dumps({"path": path, "body_polling_barrier": "100 Continue"}),
              flush=True)
        return sock

    def seed_post_narrowing_chat(self, marker):
        record = {
            "collection": "messages.chat",
            "id": "cht_00000000000000000000000501",
            "version": 1,
            "source": "messages",
            "observed_at": "2026-11-11T00:00:00Z",
            "deleted": False,
            "coverage": {"fields": "full", "missing": None},
            "data": {"display_name": marker, "participants": []},
        }
        with sqlite3.connect(self.home / "state.db", timeout=10) as db:
            db.execute("INSERT INTO records VALUES(?,?,?,?,?,?,?,?)", (
                record["collection"], record["id"], 1, "messages", 0,
                json.dumps(record).encode(), 501, record["observed_at"]))
            db.execute("INSERT INTO chat_summary(id,last_activity_at) VALUES(?,?)",
                       (record["id"], record["observed_at"]))

    def check_delayed_read(self, path, body, label):
        self.connect()
        child = None
        with (self.home / "remote/config.lock").open("r+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            try:
                child = subprocess.Popen([
                    fixture.BINARY, "remote", "enable", "--via", "cloud",
                    "--sources", "calendar", "--no-open", "--json",
                    "--home", str(self.home)], env=self.env,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

                def revoked():
                    agents = [p for p in json.loads(
                        (self.home / "credentials.json").read_text())["principals"]
                              if p["kind"] == "agent"]
                    return agents and all(p.get("revoked_at") for p in agents)

                fixture.wait(revoked)
                self.assertIsNone(child.poll())
                client, tokens = self.connect()
                sock = self.begin_delayed_post(
                    path, body, tokens["access_token"])
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)
                if child is not None:
                    try:
                        stdout, stderr = child.communicate(timeout=45)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.communicate(timeout=10)
                        raise
        self.assertEqual(child.returncode, 0, stdout + stderr)
        self.assertEqual(json.loads(
            (self.home / "remote/config.json").read_text())["cloud_sources"],
                         ["calendar"])
        fresh_status, _ = self.request(
            "POST", "/v1/op/messages.chats", "{}",
            token=tokens["access_token"])
        self.assertEqual(fresh_status, 401)
        refresh_status, _ = self.token({
            "grant_type": "refresh_token", "client_id": client,
            "refresh_token": tokens["refresh_token"]})
        self.assertEqual(refresh_status, 400)
        mcp_fresh_status, _ = self.request(
            "POST", "/mcp", json.dumps({
                "jsonrpc": "2.0", "id": 500, "method": "tools/list"}),
            token=tokens["access_token"])
        self.assertEqual(mcp_fresh_status, 401)
        marker = "AB_REVIEW_R5_CREATED_AFTER_SUCCESS_" + label
        self.seed_post_narrowing_chat(marker)
        sock.sendall(body)
        response = http.client.HTTPResponse(sock)
        response.begin()
        raw = response.read().decode()
        if "text/event-stream" in response.getheader("Content-Type", ""):
            events = [json.loads(line[5:].strip()) for line in raw.splitlines()
                      if line.startswith("data:") and line[5:].strip()]
            decoded = events[-1] if events else {}
        else:
            decoded = json.loads(raw)
        print(json.dumps({
            "test": label,
            "narrowing_succeeded": child.returncode == 0,
            "new_request_status": fresh_status,
            "new_mcp_request_status": mcp_fresh_status,
            "refresh_status": refresh_status,
            "delayed_request_status": response.status,
            "response_content_type": response.getheader("Content-Type"),
            "post_success_marker_leaked": marker in raw,
            "response": decoded,
        }), flush=True)
        self.assertNotIn(
            marker, raw,
            "a request admitted before source-save read data created after "
            "successful narrowing; the live ceiling was not rechecked")

    def test_delayed_rest_body_cannot_read_removed_source(self):
        self.check_delayed_read("/v1/op/messages.chats", b"{}", "REST")

    def test_delayed_mcp_legacy_sse_cannot_read_removed_source(self):
        body = json.dumps({
            "jsonrpc": "2.0", "id": 501, "method": "tools/call",
            "params": {"name": "messages_chats", "arguments": {}},
        }).encode()
        self.check_delayed_read("/mcp", body, "MCP_LEGACY_SSE")

    def test_delayed_mcp_compact_sse_cannot_read_removed_source(self):
        body = json.dumps({
            "jsonrpc": "2.0", "id": 502, "method": "tools/call",
            "params": {"name": "apple_read", "arguments": {
                "operation": "messages.chats", "arguments": {}}},
        }).encode()
        self.check_delayed_read("/mcp", body, "MCP_COMPACT_SSE")


class TransportReview(fixture.CloudReview):
    def test_get_mcp_does_not_open_a_persistent_sse_session(self):
        _, tokens = self.connect()
        target = urlsplit(
            self.api.config["Web"][fixture.HOST + ":443"]["Handlers"]["/"]["Proxy"])
        connection = http.client.HTTPConnection(
            target.hostname, target.port, timeout=10)
        self.addCleanup(connection.close)
        connection.request("GET", target.path + "/mcp", headers={
            "Host": fixture.HOST, "X-Forwarded-Host": fixture.HOST,
            "X-Forwarded-Proto": "https", "X-Forwarded-For": "203.0.113.1",
            "Authorization": "Bearer " + tokens["access_token"],
            "Accept": "text/event-stream",
            "MCP-Protocol-Version": "2025-03-26",
        })
        response = connection.getresponse()
        body = response.read().decode()
        print(json.dumps({"get_mcp_status": response.status, "body": body}),
              flush=True)
        self.assertEqual(response.status, 405)

    def test_unreadable_ceiling_closes_existing_stream_and_denies_mcp(self):
        _, tokens = self.connect()
        stream = self.open_stream(tokens["access_token"])
        self.append_synthetic_chat_change("AB_REVIEW_R5_BEFORE_CONFIG_ERROR")
        self.assertIn("AB_REVIEW_R5_BEFORE_CONFIG_ERROR",
                      json.dumps(self.next_data(stream)))
        (self.home / "remote/config.json").write_text("{")
        status, _ = self.request(
            "POST", "/mcp", json.dumps({
                "jsonrpc": "2.0", "id": 503, "method": "tools/list"}),
            token=tokens["access_token"])
        self.append_synthetic_chat_change("AB_REVIEW_R5_AFTER_CONFIG_ERROR")
        after = self.next_data(stream)
        print(json.dumps({"invalid_config_mcp_status": status,
                          "invalid_config_stream": after}), flush=True)
        self.assertEqual(status, 401)
        self.assertIsNone(after)


if __name__ == "__main__":
    names = [
        "test_delayed_rest_body_cannot_read_removed_source",
        "test_delayed_mcp_legacy_sse_cannot_read_removed_source",
        "test_delayed_mcp_compact_sse_cannot_read_removed_source",
    ]
    suite = unittest.TestSuite(DelayedBodyReview(name) for name in names)
    suite.addTest(TransportReview(
        "test_get_mcp_does_not_open_a_persistent_sse_session"))
    suite.addTest(TransportReview(
        "test_unreadable_ceiling_closes_existing_stream_and_denies_mcp"))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    sys.exit(not result.wasSuccessful())
