#!/usr/bin/env python3
"""Proves the documented local setup: a fresh home to a real Claude Code read, with the time measured.

usage: local-setup-proof.py --bridge BRIDGE --scratch DIR --port N [--data-home COPY] [--model MODEL] [--keep]

Without --data-home the home starts empty. With it, only state.db and ledger.db are cloned from that test copy, so
credentials and settings are still fresh. The live gateway's home and port are refused. Prints timings and counts,
never content. The browser's and the owner's parts of the sign-in are played by this script: the same requests the
sign-in page makes, then `bridge connector allow`. Claude Code's registration is project scoped in the scratch folder
and everything it added is removed at the end.
"""
import argparse, json, os, pty, re, select, shutil, sqlite3, stat, subprocess, sys, time, urllib.error, urllib.parse, urllib.request

LIVE_NAMES = ("AppleBridgeLive",)
LIVE_PORT = 7780
SERVER = "apple-bridge"
CLAUDE_STATE = os.path.expanduser("~/.claude.json")


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
    return options


def clean_environment(extra=None):
    env = {key: value for key, value in os.environ.items() if not key.startswith("CLAUDE")}
    env["APPLE_BRIDGE_SECRET_STORE"] = "file"
    env.update(extra or {})
    return env


class Proof:
    def __init__(self, options):
        self.options = options
        self.scratch = os.path.abspath(options.scratch)
        self.home = os.path.join(self.scratch, "home")
        self.project = os.path.join(self.scratch, "project")
        self.browser_log = os.path.join(self.scratch, "browser-url")
        self.origin = "http://127.0.0.1:%d" % options.port
        self.marks = {}
        self.began = None
        self.tools = os.path.join(self.scratch, "tools")

    def mark(self, name):
        self.marks[name] = round(time.monotonic() - self.began, 2)
        print("%-34s %8.2f s" % (name, self.marks[name]), flush=True)

    def bridge(self, *arguments, check=True):
        result = subprocess.run([self.options.bridge, *arguments, "--home", self.home], env=clean_environment(), capture_output=True, text=True, cwd=self.project, stdin=subprocess.DEVNULL)
        if check and result.returncode != 0:
            raise SystemExit("bridge %s failed: %s" % (arguments[0], result.stderr.strip()))
        return result

    def prepare(self):
        if os.path.exists(self.scratch):
            sys.exit("%s already exists; give a new folder" % self.scratch)
        os.makedirs(self.project)
        os.makedirs(self.tools)
        os.symlink(os.path.abspath(self.options.bridge), os.path.join(self.tools, "bridge"))
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

    def approve_project_server(self):
        env = clean_environment()
        name = "t7-proof-%d" % os.getpid()
        socket = ["tmux", "-L", name]
        launch = ["env", "-u", "CLAUDECODE", "claude", "--model", self.options.model]
        subprocess.run(socket + ["-f", "/dev/null", "new-session", "-d", "-x", "120", "-y", "40", "-s", "proof", "-c", self.project, *launch], env=env, check=True)
        try:
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline:
                time.sleep(1)
                screen = subprocess.run(socket + ["capture-pane", "-p", "-t", "proof"], capture_output=True, text=True).stdout
                if "I trust this folder" in screen:
                    subprocess.run(socket + ["send-keys", "-t", "proof", "Down"])
                    time.sleep(0.3)
                    subprocess.run(socket + ["send-keys", "-t", "proof", "Enter"])
                elif "Use this MCP server" in screen:
                    subprocess.run(socket + ["send-keys", "-t", "proof", "Up"])
                    time.sleep(0.3)
                    subprocess.run(socket + ["send-keys", "-t", "proof", "Up"])
                    time.sleep(0.3)
                    subprocess.run(socket + ["send-keys", "-t", "proof", "Enter"])
                    time.sleep(3)
                    return
            raise SystemExit("Claude Code's approval screen never appeared")
        finally:
            subprocess.run(socket + ["kill-server"], capture_output=True)

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

    def link_with_terminal(self):
        env = clean_environment({"BROWSER": os.path.join(self.tools, "browser")})
        pid, terminal = pty.fork()
        if pid == 0:
            os.chdir(self.project)
            os.execve(self.options.bridge, [self.options.bridge, "link", "claude", "--scope", "project", "--home", self.home], env)
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
            if request is None and os.path.exists(self.browser_log):
                with open(self.browser_log) as lines:
                    url = lines.readline().strip()
                if url:
                    request = self.browse(url)
                    self.mark("sign-in page reached the gateway")
            if request and not allowed and b"connector allow" in output:
                self.bridge("connector", "allow", "--request", request["id"], "--code", request["code"])
                allowed = True
                self.mark("request allowed")
                self.poll(request["id"])
        _, status = os.waitpid(pid, 0)
        if os.waitstatus_to_exitcode(status) != 0 or not allowed:
            raise SystemExit("bridge link failed: %s" % re.sub(rb"[?&](code|state|code_challenge)=[^&\s]+", b"", output)[-600:].decode("utf8", "replace"))

    def ask_claude(self, prompt, tools):
        command = ["claude", "-p", prompt, "--model", self.options.model, "--no-session-persistence", "--output-format", "json", "--permission-mode", "dontAsk", "--allowedTools", *tools]
        env = clean_environment({"PATH": self.tools + os.pathsep + os.environ.get("PATH", "")})
        result = subprocess.run(command, cwd=self.project, env=env, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=240)
        if result.returncode != 0:
            raise SystemExit("claude failed with %d" % result.returncode)
        answer = json.loads(result.stdout)
        return answer.get("result", ""), answer.get("is_error", False), answer.get("permission_denials", [])

    def run(self):
        expected = self.expected_counts()
        self.prepare()
        before = claude_state()
        try:
            self.began = time.monotonic()
            up = json.loads(self.bridge("up", "--api-addr", "127.0.0.1:%d" % self.options.port, "--peer-addr", "127.0.0.1:0", "--json").stdout)
            if up["remote"] != "off":
                raise SystemExit("remote must be off")
            self.mark("bridge up")
            stats = json.loads(self.bridge("stats", "--json").stdout)
            self.mark("bridge stats")
            seen = {collection: count for source in stats["sources"] for collection, count in source["counts"].items()}
            first_link = self.bridge("link", "claude", "--scope", "project")
            if "approve project servers" not in first_link.stdout:
                raise SystemExit("expected Claude Code's project approval step")
            self.mark("bridge link registered")
            paused = time.monotonic()
            self.approve_project_server()
            approval = round(time.monotonic() - paused, 2)
            self.began += time.monotonic() - paused
            print("%-34s %8.2f s (not counted; only --scope project needs it)" % ("project approval in Claude Code", approval), flush=True)
            self.link_with_terminal()
            self.mark("signed in")
            collection = "reminders.list" if expected else None
            if collection:
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
            agents = json.loads(self.bridge("agent", "list").stdout)
            terminal_reads = {}
            for what in ("reminders", "contacts", "chats", "calendar", "notes"):
                terminal_reads[what] = self.bridge("read", what, "--json", check=False).returncode == 0
            report = {
                "data": "test copy" if expected else "empty home",
                "seconds": self.marks,
                "project_approval_seconds": approval,
                "up_started": up["started"],
                "collections_compared": len(expected),
                "stats_match_source": all(seen.get(key, 0) == value for key, value in expected.items()),
                "read_matched": read_ok,
                "read_count": got,
                "read_expected": want,
                "agent_stats_matched": stats_ok,
                "oauth_agents_on_gateway": sum(1 for agent in agents if not agent.get("revoked_at")),
                "terminal_reads": terminal_reads,
            }
            print(json.dumps(report, indent=1))
            return read_ok and stats_ok and report["stats_match_source"]
        finally:
            self.cleanup(before)

    def cleanup(self, before):
        self.bridge("unlink", "claude", "--scope", "project", check=False)
        self.bridge("down", check=False)
        remove_project_entry(self.project)
        after = claude_state()
        print("claude config servers unchanged:", before["servers"] == after["servers"], "project entry removed:", self.project not in after["projects"])
        if not self.options.keep:
            shutil.rmtree(self.scratch, ignore_errors=True)


def claude_state():
    with open(CLAUDE_STATE) as source:
        data = json.load(source)
    return {"servers": sorted(data.get("mcpServers", {})), "projects": sorted(data.get("projects", {}))}


def remove_project_entry(project):
    with open(CLAUDE_STATE) as source:
        data = json.load(source)
    for key in (project, os.path.realpath(project)):
        data.get("projects", {}).pop(key, None)
    temporary = CLAUDE_STATE + ".t7-proof"
    with open(temporary, "w") as output:
        json.dump(data, output, indent=2)
    os.chmod(temporary, os.stat(CLAUDE_STATE).st_mode & 0o777)
    os.replace(temporary, CLAUDE_STATE)
    transcripts = os.path.expanduser("~/.claude/projects/" + re.sub(r"[^A-Za-z0-9]", "-", os.path.realpath(project)))
    shutil.rmtree(transcripts, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(0 if Proof(parse()).run() else 1)
