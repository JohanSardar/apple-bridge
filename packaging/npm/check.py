#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build import ADHOC, APP, EXECUTABLES, code_paths, run, signature_problems, version_problems

OPEN_STUB = """#!/bin/sh
echo "$$" >> "$(dirname "$0")/pids"
printf '%s\\n' "$*" >> "$(dirname "$0")/calls"
exec sleep 300
"""


class Report:
    def __init__(self):
        self.failures = []

    def expect(self, name, passed, detail=""):
        print(f"{'pass' if passed else 'FAIL'}  {name}" + (f"  {detail}" if detail and not passed else ""))
        if not passed:
            self.failures.append(name)

    def transcript(self, command, result):
        print(f"$ {command}\n{result.stdout}{result.stderr}[exit {result.returncode}]")


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def fingerprints(tree):
    return {str(path.relative_to(tree)): (hashlib.sha256(path.read_bytes()).hexdigest(), os.access(path, os.X_OK)) for path in sorted(tree.rglob("*")) if path.is_file()}


def code_facts(app):
    facts = {}
    for name, path in code_paths(app).items():
        requirement = run("codesign", "-d", "-r-", path).stdout
        details = run("codesign", "-dvvv", path).stderr
        facts[name] = {
            "requirement": next(line for line in requirement.splitlines() if line.startswith("designated")),
            "cdhash": next(line for line in details.splitlines() if line.startswith("CDHash=")),
        }
    return facts


def inspect_tarball(tarball, report):
    with tarfile.open(tarball) as archive:
        members = archive.getmembers()
        definition = json.load(archive.extractfile("package/package.json"))
    report.expect("the tarball holds only regular files and folders", all(member.isfile() or member.isdir() for member in members))
    report.expect("the package has no install scripts", not definition.get("scripts"))
    report.expect("the package only installs on Apple silicon Macs", definition.get("os") == ["darwin"] and definition.get("cpu") == ["arm64"])
    report.expect("the command is bridge", definition.get("bin") == {"bridge": "bin/bridge"})
    return definition


def inspect_install(prefix, name, source_app, identity, facts, report, label):
    package = prefix / "lib/node_modules" / name
    app = package / APP
    link = prefix / "bin/bridge"
    report.expect(f"{label}: bin/bridge links to the package launcher", link.is_symlink() and link.resolve() == (package / "bin/bridge").resolve())
    report.expect(f"{label}: every app file is byte-identical with the same executable bit", fingerprints(app) == fingerprints(source_app))
    report.expect(f"{label}: signature, identifier, leaf and hardened runtime", not signature_problems(app, identity), "; ".join(signature_problems(app, identity)))
    installed = code_facts(app)
    report.expect(f"{label}: codesign -d -r- and CDHash match the app before packing", installed == facts, f"{installed} != {facts}")
    attributes = run("xattr", "-lr", app).stdout
    report.expect(f"{label}: no quarantine attribute", "com.apple.quarantine" not in attributes)
    return app


def install(tarball, prefix, environment, flags=()):
    return run("npm", "install", "-g", "--prefix", prefix, "--offline", "--no-audit", "--no-fund", *flags, tarball, env=environment)


class World:
    def __init__(self, work, prefix):
        self.work = work
        self.home = work / "data"
        self.user_home = work / "home"
        self.user_home.mkdir()
        self.stub = work / "stub"
        self.stub.mkdir()
        script = self.stub / "open"
        script.write_text(OPEN_STUB)
        script.chmod(0o755)
        self.environment = {
            **os.environ,
            "HOME": str(self.user_home),
            "APPLE_BRIDGE_SECRET_STORE": "file",
            "PATH": os.pathsep.join([str(self.stub), str(prefix / "bin"), os.environ["PATH"]]),
        }
        if shutil.which("open", path=self.environment["PATH"]) != str(script):
            raise SystemExit("Refusing to run: the stand-in for open is not first on PATH, so the real Helper could launch.")

    def bridge(self, report, *arguments, show=True):
        result = run("bridge", *arguments, env=self.environment, timeout=120)
        if show:
            report.transcript(" ".join(["bridge", *arguments]), result)
        return result

    def launch_calls(self, wanted):
        calls = self.stub / "calls"
        for _ in range(150):
            text = calls.read_text() if calls.exists() else ""
            if wanted in text:
                break
            time.sleep(0.1)
        return text

    def log_tail(self):
        log = self.home / "gateway.log"
        return log.read_text()[-800:] if log.exists() else "(no gateway log)"

    def stop_stubs(self):
        pids = self.stub / "pids"
        for pid in pids.read_text().split() if pids.exists() else []:
            try:
                os.kill(int(pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass


def exercise(world, app, port, report, label):
    home = str(world.home)
    version = world.bridge(report, "--version")
    report.expect(f"{label}: bridge --version", version.returncode == 0)
    up = world.bridge(report, "up", "--home", home, "--api-addr", f"127.0.0.1:{port}", "--peer-addr", "127.0.0.1:0")
    report.expect(f"{label}: bridge up", up.returncode == 0 and "is on" in up.stdout)
    report.expect(f"{label}: bridge up found the installed app", "no Helper app found" not in up.stdout)
    wanted = f"-a {app.resolve()} --args session"
    report.expect(f"{label}: the gateway asks open for the installed app", wanted in world.launch_calls(wanted), f"open calls: {world.launch_calls(wanted)!r}. Gateway log: {world.log_tail()}")
    report.expect(f"{label}: the app itself was never launched", run("pgrep", "-f", f"{app.resolve()}/Contents/MacOS/AppleBridgeHelper").returncode == 1)
    stats = world.bridge(report, "stats", "--home", home)
    report.expect(f"{label}: bridge stats", stats.returncode == 0)
    down = world.bridge(report, "down", "--home", home)
    report.expect(f"{label}: bridge down", down.returncode == 0 and "stopped" in down.stdout)


def replaced_while_running(world, tarball, port, app, source_app, identity, facts, name, prefix, report):
    home = str(world.home)
    up = world.bridge(report, "up", "--home", home, "--api-addr", f"127.0.0.1:{port}", "--peer-addr", "127.0.0.1:0")
    report.expect("update: the gateway runs before the update", up.returncode == 0)
    report.expect("update: npm can replace the package under a running gateway", install(tarball, prefix, world.environment).returncode == 0)
    inspect_install(prefix, name, source_app, identity, facts, report, "update")
    stats = world.bridge(report, "stats", "--home", home)
    report.expect("update: the running gateway still answers", stats.returncode == 0)
    down = world.bridge(report, "down", "--home", home)
    report.expect("update: bridge down stops the older process", down.returncode == 0 and "stopped" in down.stdout)


def other_install_running(world, port, report):
    home = str(world.home)
    command = ["bridge", "start", "--role", "gateway", "--home", home, "--api-addr", f"127.0.0.1:{port}", "--peer-addr", "127.0.0.1:0"]
    gateway = subprocess.Popen(command, env=world.environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(600):
            if gateway.poll() is not None or run("curl", "-fs", f"http://127.0.0.1:{port}/v1/health").returncode == 0:
                break
            time.sleep(0.1)
        up = world.bridge(report, "up", "--home", home)
        report.expect("other install: bridge up reports the running gateway", up.returncode == 0 and "was already on" in up.stdout)
        down = world.bridge(report, "down", "--home", home)
        report.expect("other install: bridge down leaves it running", down.returncode != 0 and "not started by" in down.stderr + down.stdout)
        report.expect("other install: it is still running", gateway.poll() is None)
    finally:
        gateway.terminate()
        gateway.wait(timeout=30)


def stapling(app, report, required):
    stapled = run("xcrun", "stapler", "validate", app).returncode == 0
    assessed = run("spctl", "-a", "-vv", "--type", "execute", app).returncode == 0
    print(f"note  stapled={stapled} gatekeeper_accepted={assessed}")
    if required:
        report.expect("the installed app is stapled and Gatekeeper accepts it", stapled and assessed)


def main():
    parser = argparse.ArgumentParser(description="Install the tarball into throwaway folders and check signatures and bridge. It never launches the app: a stand-in replaces open, and the data folder and HOME are throwaway.")
    parser.add_argument("--tarball", required=True, type=Path)
    parser.add_argument("--app", required=True, type=Path, help="The signed app the tarball was made from")
    parser.add_argument("--identity", required=True, help=f"SHA-1 fingerprint the app is signed with, or {ADHOC} for ad hoc")
    parser.add_argument("--require-notarized", action="store_true")
    args = parser.parse_args()
    source_app = args.app.resolve()
    tarball = args.tarball.resolve()
    report = Report()
    definition = inspect_tarball(tarball, report)
    name = definition["name"]
    print(f"note  {name}@{definition['version']} {tarball.stat().st_size} bytes, private={definition.get('private', False)}")
    facts = code_facts(source_app)
    for code, fact in facts.items():
        print(f"note  {code} {fact['requirement']}")
    work = Path(tempfile.mkdtemp(prefix="ab-npm-", dir="/tmp"))
    worlds = []
    try:
        for label, flags in [("default", ()), ("ignore-scripts", ("--ignore-scripts",))]:
            scenario = work / label
            scenario.mkdir()
            prefix = scenario / "prefix"
            world = World(scenario, prefix)
            worlds.append(world)
            installed = install(tarball, prefix, world.environment, flags)
            report.transcript(f"npm install -g --prefix <throwaway> {' '.join(flags)} <tarball>", installed)
            report.expect(f"{label}: npm install", installed.returncode == 0)
            if installed.returncode != 0:
                continue
            app = inspect_install(prefix, name, source_app, args.identity, facts, report, label)
            problems = version_problems(app, definition["version"])
            report.expect(f"{label}: the package, bridge and the app report one version", not problems, "; ".join(problems))
            reported = world.bridge(report, "--version", show=False)
            report.expect(f"{label}: the bridge command reports the package version", reported.stdout.strip() == f"bridge {definition['version']}", reported.stdout.strip())
            exercise(world, app, free_port(), report, label)
            if label == "default":
                stapling(app, report, args.require_notarized)
                replaced_while_running(world, tarball, free_port(), app, source_app, args.identity, facts, name, prefix, report)
                other_install_running(world, free_port(), report)
            removed = run("npm", "uninstall", "-g", "--prefix", prefix, name, env=world.environment)
            report.expect(f"{label}: npm uninstall removes the command and the app", removed.returncode == 0 and not (prefix / "bin/bridge").exists() and not app.exists())
    finally:
        for world in worlds:
            world.stop_stubs()
        shutil.rmtree(work, ignore_errors=True)
    print(f"{len(report.failures)} failed" + (": " + ", ".join(report.failures) if report.failures else ""))
    return 1 if report.failures else 0


if __name__ == "__main__":
    sys.exit(main())
