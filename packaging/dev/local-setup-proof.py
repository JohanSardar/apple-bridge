#!/usr/bin/env python3
"""Proves the documented local setup: a fresh home to a real Claude Code read, with the time measured.

usage: local-setup-proof.py --bridge BRIDGE --scratch DIR --port N [--data-home COPY] [--model MODEL] [--keep]

Without --data-home the home starts empty. With it, only state.db and ledger.db are cloned from that test copy, so
credentials and settings are still fresh. The live gateway's home and port are refused. Prints timings and counts,
never content. Claude Code and Codex run under a throwaway HOME and CODEX_HOME inside the scratch folder, so the real
~/.claude.json and ~/.codex are never written. Claude Code reaches its own account sign-in through a link to the login
Keychain folder. `bridge link` registers `bridge mcp`; the first time each agent starts it, the owner's Allow click is
played by this script with `bridge connectors allow`. Give a bridge binary outside an app bundle, so no Allow panel opens.
"""
import argparse, json, os, re, shutil, sqlite3, subprocess, sys, threading, time

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
    if os.path.exists(os.path.join(os.path.dirname(os.path.dirname(options.bridge)), "Library", "LaunchAgents")):
        sys.exit("give a bridge binary outside an app bundle, so no Allow panel opens on this screen")
    return options


class Proof:
    def __init__(self, options):
        self.options = options
        self.scratch = os.path.abspath(options.scratch)
        self.home = os.path.join(self.scratch, "home")
        self.project = os.path.join(self.scratch, "project")
        self.agent_home = os.path.join(self.scratch, "agent-home")
        self.tools = os.path.join(self.scratch, "tools")
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

    def link(self, client):
        self.bridge("link", client)
        self.mark("%s: linked" % client)

    def allow_first_ask(self, client, finished):
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline and not finished.is_set():
            listed = json.loads(self.bridge("connectors", "list", "--json").stdout)
            for ask in listed["connectors"]:
                self.bridge("connectors", "allow", "--request", ask["id"], "--code", ask["code"])
                self.mark("%s: allowed on the Mac" % client)
                return True
            time.sleep(0.5)
        return False

    def with_allow(self, client, work):
        finished = threading.Event()
        result = {}
        watcher = threading.Thread(target=lambda: result.update(allowed=self.allow_first_ask(client, finished)))
        watcher.start()
        try:
            return work(), result
        finally:
            finished.set()
            watcher.join()

    def ask_claude(self, prompt, tools):
        command = ["claude", "-p", prompt, "--model", self.options.model, "--no-session-persistence", "--output-format", "json", "--permission-mode", "dontAsk", "--allowedTools", *tools]
        result = subprocess.run(command, cwd=self.project, env=self.environment(), capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=240)
        if result.returncode != 0:
            raise SystemExit("claude failed with %d" % result.returncode)
        answer = json.loads(result.stdout)
        return answer.get("result", ""), answer.get("is_error", False), answer.get("permission_denials", [])

    def prove_codex(self):
        began = time.monotonic()
        self.link("codex")
        listed = subprocess.run(["codex", "mcp", "get", SERVER], env=self.environment(), capture_output=True, text=True, stdin=subprocess.DEVNULL).stdout
        return {"registered": self.options.bridge in listed and "mcp" in listed, "seconds": round(time.monotonic() - began, 2)}

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
            self.link("claude")
            if expected:
                collection = "reminders.list"
                prompt = "Call the apple-bridge apple_read tool with operation bridge.list_records and arguments {\"collection\": \"%s\", \"limit\": 100}. Reply with only the number of records it returned, as digits." % collection
                tools = ["mcp__%s__apple_read" % SERVER]
                want = expected.get(collection, 0)
            else:
                prompt = "Call the apple-bridge apple_read tool with operation bridge.list_sources and arguments {}. Reply with only the number of records it returned, as digits."
                tools = ["mcp__%s__apple_read" % SERVER]
                want = 0
            (reply, failed, denied), watched = self.with_allow("claude", lambda: self.ask_claude(prompt, tools))
            got = re.sub(r"\D", "", reply)
            self.mark("first real read by Claude Code")
            read_ok = bool(watched.get("allowed")) and not failed and not denied and got == str(want)
            stats_prompt = "Run this command exactly: bridge stats --json --home %s . Reply with only the value of its top-level total field, as digits." % self.home
            reply, failed, denied = self.ask_claude(stats_prompt, ["Bash(bridge stats:*)"])
            asks_left = len(json.loads(self.bridge("connectors", "list", "--json").stdout)["connectors"])
            stats_ok = not failed and not denied and re.sub(r"\D", "", reply) == str(stats["total"])
            self.mark("Claude Code read bridge stats")
            codex = self.prove_codex()
            agents = json.loads(self.bridge("agent", "list", "--json").stdout)
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
                "agents_on_gateway": sum(1 for agent in agents if not agent.get("revoked_at")),
                "asks_after_first_allow": asks_left,
                "terminal_reads": terminal_reads,
            }
            print(json.dumps(report, indent=1))
            return read_ok and stats_ok and report["stats_match_source"] and codex["registered"]
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
