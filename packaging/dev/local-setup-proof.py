#!/usr/bin/env python3
"""Proves the documented local setup: a fresh home to a real Claude Code read, with the time measured.

usage: local-setup-proof.py --bridge BRIDGE --scratch DIR --port N [--data-home COPY] [--model MODEL] [--keep]

Without --data-home the home starts empty. With it, only state.db and ledger.db are cloned from that test copy, so
credentials and settings are still fresh. The live gateway's home and port are refused. Prints timings and counts,
never content. Claude Code and Codex run under a throwaway HOME and CODEX_HOME inside the scratch folder, so the real
~/.claude.json and ~/.codex are never written. Claude Code reaches its sign-in through a link to the login Keychain
folder; its sign-in to the test gateway is removed from there at the end. The browser's and the owner's parts of the
sign-in are played by this script: the same requests the sign-in page makes, then `bridge connector allow`.
"""
import argparse, json, os, pty, re, select, shutil, sqlite3, stat, subprocess, sys, time, urllib.error, urllib.parse, urllib.request

LIVE_NAMES = ("AppleBridgeLive",)
LIVE_PORT = 7780
SERVER = "apple-bridge"


def parse():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bridge", required=True)
    parser.add_argument("--scratch", required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--data-home")
    parser.add_argument("--model", default="claude-haiku-5-5")
    parser.add_argument("--keep", action="store_true")
    options = parser.parse_args()
    if options.port == LIVE_PORT:
        sys.exit("7780 belongs to the live gateway; choose another port")
    for path in (options.scratch, options.data_home or ""):
        if any(name in path for name in LIVE_NAMES):
            sys.exit("refusing the live home")
    options.bridge = os.path.abspath(options.bridge)
    return options


class Proof:
    def __init__(self, options):
        self.options = options
        self.scratch = os.path.abspath(options.scratch)
        self.home = os.path.join(self.scratch, "home")
        self.project = os.path.join(self.scratch, "project")
        self.agent_home = os.path.join(self.scratch, "agent-home")
        self.tools = os.path.join(self.scratch, "tools")
        self.browser_log = os.path.join(self.scratch, "browser-url")
        self.origin = "http://127.0.0.1:%d" % options.port
        self.marks = {}
        self.began = None

    def environment(self, extra=None):
        env = {key: os.environ[key] for key in ("USER", "LOGNAME", "TMPDIR", "LANG") if key in os.environ}
        env.update(
            PATH=self.tools + os.pathsep + os.environ.get("PATH", ""),
            HOME=self.agent_home,
            CODEX_HOME=os.path.join(self.agent_home, ".codex"),
            TERM="xterm",
            APPLE_BRIDGE_SECRET_STORE="file",
            BROWSER=os.path.join(self.tools, "browser"),
        )
        env.update(extra or {})
        return env

    def mark(self, name):
        self.marks[name] = round(time.monotonic() - self.began, 2)
        print("%-34s %8.2f s" % (name, self.marks[name]), flush=True)

    def bridge(self, *arguments, check=True):
        result = subprocess.run([self.options.bridge, *arguments, "--home", self.home], env=self.environment(), capture_output=True, text=True, cwd=self.project, stdin=subprocess.DEVNULL)
        if check and result.returncode != 0:
            raise SystemExit("bridge %s failed: %s" % (arguments[0], result.stderr.strip()))
        return result

    def prepare(self):
        if os.path.exists(self.scratch):
            sys.exit("%s already exists; give a new folder" % self.scratch)
        os.makedirs(self.project)
        os.makedirs(self.tools)
        os.makedirs(os.path.join(self.agent_home, "Library"))
        os.makedirs(os.path.join(self.agent_home, ".codex"))
        os.symlink(os.path.expanduser("~/Library/Keychains"), os.path.join(self.agent_home, "Library", "Keychains"))
        with open(os.path.join(self.agent_home, ".claude.json"), "w") as output:
            json.dump({"hasCompletedOnboarding": True}, output)
        os.symlink(self.options.bridge, os.path.join(self.tools, "bridge"))
        shim = os.path.join(self.tools, "browser")
        with open(shim, "w") as output:
            output.write('#!/bin/sh\nprintf "%%s\\n" "$1" >> %s\n' % json.dumps(self.browser_log))
        os.chmod(shim, stat.S_IRWXU)
        if self.options.data_home:
            os.makedirs(self.home, mode=0o700)
            for name in os.listdir(self.options.data_home):
                if name.startswith(("state.db", "ledger.db")):
                    subprocess.run(["cp", "-c", os.path.join(self.options.data_home, name), os.path.join(self.home, name)], check=True)

    def expected_counts(self):
        if not self.options.data_home:
            return {}
        db = sqlite3.connect("file:%s?mode=ro" % os.path.join(self.options.data_home, "state.db"), uri=True, timeout=30)
        rows = db.execute("select collection,count(*) from records where deleted=0 and collection not like 'system.%' group by collection").fetchall()
        db.close()
        return dict(rows)

    def browse(self, url):
        query = urllib.parse.urlsplit(url).query
        with urllib.request.urlopen(url, timeout=30) as page:
            if page.status != 200:
                raise SystemExit("sign-in page answered %s" % page.status)
        request = urllib.request.Request(self.origin + "/oauth/pending?" + query, method="POST", headers={"Origin": self.origin})
        with urllib.request.urlopen(request, timeout=30) as answer:
            return json.load(answer)

    def poll(self, request_id):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            time.sleep(1)
            try:
                with urllib.request.urlopen(self.origin + "/oauth/pending/" + urllib.parse.quote(request_id), timeout=30) as answer:
                    redirect = json.load(answer).get("redirect")
            except urllib.error.HTTPError as error:
                if error.code == 429:
                    continue
                raise
            if redirect:
                with urllib.request.urlopen(redirect, timeout=30) as finished:
                    return finished.status
        raise SystemExit("the sign-in request was never decided")

    def sign_in_url(self, output):
        if os.path.exists(self.browser_log):
            with open(self.browser_log) as lines:
                url = lines.readline().strip()
            if url:
                return url
        flat = re.sub(rb"\x1b\][^\x07\x1b]*(\x07|\x1b\\)", b"", output).replace(b"\r\n", b"")
        found = re.search(rb"http://127\.0\.0\.1:\d+/oauth/authorize\?[^\s\x1b]+", flat)
        return found.group(0).decode() if found else None

    def link_with_terminal(self, client):
        if os.path.exists(self.browser_log):
            os.remove(self.browser_log)
        env = self.environment()
        pid, terminal = pty.fork()
        if pid == 0:
            os.chdir(self.project)
            os.execve(self.options.bridge, [self.options.bridge, "link", client, "--home", self.home], env)
        output = b""
        allowed = False
        request = None
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            ready, _, _ = select.select([terminal], [], [], 0.2)
            if ready:
                try:
                    chunk = os.read(terminal, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                output += chunk
            if request is None:
                url = self.sign_in_url(output)
                if url:
                    request = self.browse(url)
                    self.mark("%s: sign-in page opened" % client)
            if request and not allowed and b"connector allow" in output:
                self.bridge("connector", "allow", "--request", request["id"], "--code", request["code"])
                allowed = True
                self.mark("%s: request allowed" % client)
                self.poll(request["id"])
        _, status = os.waitpid(pid, 0)
        if os.waitstatus_to_exitcode(status) != 0 or not allowed:
            raise SystemExit("bridge link %s failed with status %s" % (client, status))

    def ask_claude(self, prompt, tools):
        command = ["claude", "-p", prompt, "--model", self.options.model, "--no-session-persistence", "--output-format", "json", "--permission-mode", "dontAsk", "--allowedTools", *tools]
        result = subprocess.run(command, cwd=self.project, env=self.environment(), capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=240)
        if result.returncode != 0:
            raise SystemExit("claude failed with %d" % result.returncode)
        answer = json.loads(result.stdout)
        return answer.get("result", ""), answer.get("is_error", False), answer.get("permission_denials", [])

    def prove_codex(self):
        began = time.monotonic()
        self.link_with_terminal("codex")
        listed = subprocess.run(["codex", "mcp", "list"], env=self.environment(), capture_output=True, text=True, stdin=subprocess.DEVNULL).stdout
        line = next((line for line in listed.splitlines() if line.startswith(SERVER)), "")
        return {"signed_in": "OAuth" in line, "seconds": round(time.monotonic() - began, 2), "real_read": False}

    def run(self):
        expected = self.expected_counts()
        self.prepare()
        try:
            self.began = time.monotonic()
            up = json.loads(self.bridge("up", "--api-addr", "127.0.0.1:%d" % self.options.port, "--peer-addr", "127.0.0.1:0", "--json").stdout)
            if up["remote"] != "off":
                raise SystemExit("remote must be off")
            self.mark("bridge up")
            stats = json.loads(self.bridge("stats", "--json").stdout)
            self.mark("bridge stats")
            seen = {collection: count for source in stats["sources"] for collection, count in source["counts"].items()}
            self.link_with_terminal("claude")
            self.mark("claude: signed in")
            if expected:
                collection = "reminders.list"
                prompt = "Call the apple-bridge records_query tool with collection %s and limit 200. Reply with only the number of records it returned, as digits." % collection
                reply, failed, denied = self.ask_claude(prompt, ["mcp__%s__records_query" % SERVER])
                want = expected.get(collection, 0)
            else:
                prompt = "Call the apple-bridge sources_status tool. Reply with only the number of records it returned, as digits."
                reply, failed, denied = self.ask_claude(prompt, ["mcp__%s__sources_status" % SERVER])
                want = 0
            got = re.sub(r"\D", "", reply)
            self.mark("first real read by Claude Code")
            read_ok = not failed and not denied and got == str(want)
            stats_prompt = "Run this command exactly: bridge stats --json --home %s . Reply with only the value of its top-level total field, as digits." % self.home
            reply, failed, denied = self.ask_claude(stats_prompt, ["Bash(bridge stats:*)"])
            stats_ok = not failed and not denied and re.sub(r"\D", "", reply) == str(stats["total"])
            self.mark("Claude Code read bridge stats")
            codex = self.prove_codex()
            agents = json.loads(self.bridge("agent", "list").stdout)
            terminal_reads = {}
            for what in ("reminders", "contacts", "chats", "calendar", "notes"):
                terminal_reads[what] = self.bridge("read", what, "--json", check=False).returncode == 0
            report = {
                "data": "test copy" if expected else "empty home",
                "seconds": self.marks,
                "collections_compared": len(expected),
                "stats_match_source": all(seen.get(key, 0) == value for key, value in expected.items()),
                "read_matched": read_ok,
                "read_count": got,
                "read_expected": want,
                "agent_stats_matched": stats_ok,
                "codex": codex,
                "oauth_agents_on_gateway": sum(1 for agent in agents if not agent.get("revoked_at")),
                "terminal_reads": terminal_reads,
            }
            print(json.dumps(report, indent=1))
            return read_ok and stats_ok and report["stats_match_source"] and codex["signed_in"]
        finally:
            self.cleanup()

    def cleanup(self):
        self.bridge("unlink", "claude", check=False)
        self.bridge("unlink", "codex", check=False)
        self.bridge("down", check=False)
        if not self.options.keep:
            shutil.rmtree(self.scratch, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(0 if Proof(parse()).run() else 1)
