#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def run(*command, **options):
    result = subprocess.run(command, capture_output=True, text=True, **options)
    if result.returncode != 0:
        raise RuntimeError(f"{command[0]} failed: {result.stderr.strip()}")
    return result


class Signing:
    def __init__(self, work):
        self.work = work
        self.keychain = work / "identity-test.keychain-db"
        self.password = "identity-test"
        run("security", "create-keychain", "-p", self.password, str(self.keychain))
        run("security", "set-keychain-settings", "-lut", "21600", str(self.keychain))
        run("security", "unlock-keychain", "-p", self.password, str(self.keychain))
        self.search(lambda current: current + [str(self.keychain)])

    @staticmethod
    def search(change):
        listed = run("security", "list-keychains", "-d", "user").stdout
        current = [line.strip().strip('"') for line in listed.splitlines() if line.strip()]
        run("security", "list-keychains", "-d", "user", "-s", *change(current))

    def identity(self, name):
        config = self.work / f"{name}.cnf"
        config.write_text(f"""[req]
distinguished_name = name
x509_extensions = signing
prompt = no
[name]
CN = {name}
[signing]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
""")
        key, certificate, bundle = (self.work / f"{name}.{suffix}" for suffix in ("key", "pem", "p12"))
        run("openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-x509", "-days", "2", "-config", str(config), "-keyout", str(key), "-out", str(certificate))
        run("openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(certificate), "-out", str(bundle), "-passout", f"pass:{self.password}", "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1")
        run("security", "import", str(bundle), "-k", str(self.keychain), "-P", self.password, "-T", "/usr/bin/codesign")
        run("security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", self.password, str(self.keychain))
        fingerprint = run("openssl", "x509", "-in", str(certificate), "-noout", "-fingerprint", "-sha1").stdout
        return fingerprint.strip().split("=")[1].replace(":", "")

    def sign(self, path, identity, identifier, hardened=True):
        command = ["codesign", "--force", "--keychain", str(self.keychain), "--sign", identity, "--timestamp=none", "--identifier", identifier]
        if hardened:
            command += ["--options", "runtime"]
        run(*command, str(path))

    def close(self):
        self.search(lambda current: [entry for entry in current if Path(entry).name != self.keychain.name])
        subprocess.run(["security", "delete-keychain", str(self.keychain)], capture_output=True)


def adhoc(path, identifier):
    run("codesign", "--force", "--sign", "-", "--options", "runtime", "--identifier", identifier, str(path))


def application(destination, executable, name):
    macos = destination / "Contents/MacOS"
    macos.mkdir(parents=True)
    shutil.copy2(executable, macos / name)
    (destination / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable": name, "CFBundleIdentifier": "dev.applebridge.identity-test", "CFBundlePackageType": "APPL"}))
    return macos


class Listener:
    def __init__(self, binary, path):
        self.process = subprocess.Popen([str(binary), "listen", str(path)], stdout=subprocess.PIPE, text=True)
        if self.process.stdout.readline().strip() != "listening":
            raise RuntimeError("fixture listener did not start")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.process.kill()
        self.process.wait()


def private_directory(path):
    path.mkdir(parents=True, exist_ok=True)
    path.chmod(0o700)
    return path


def synthetic_checks(output, signing, trusted, other, lab, failures):
    work = private_directory(signing.work / "checks")
    fixture = output / "HelperIdentityFixture"
    app = work / "Harness.app"
    macos = application(app, output / "HelperIdentityChecks", "HelperIdentityChecks")
    harness = macos / "HelperIdentityChecks"
    session = macos / "ab-session"
    bridge = macos / "bridge"
    secret = work / "secret"
    secret.write_text("synthetic")
    secret.chmod(0o600)

    def install(path, identity, identifier, hardened=True):
        path.unlink(missing_ok=True)
        shutil.copy2(fixture, path)
        if identity == "adhoc":
            adhoc(path, identifier)
        else:
            signing.sign(path, identity, identifier, hardened)

    signing.sign(harness, trusted, "harness")

    def launch(name, peer_identity, peer_identifier, session_identity, expected, peer_hardened=True, session_identifier="ab-session", environment=None):
        scenario = private_directory(work / name)
        peer = scenario / "bridge"
        install(peer, peer_identity, peer_identifier, peer_hardened)
        install(session, session_identity, session_identifier)
        directory = private_directory(Path(tempfile.mkdtemp(prefix="ab-id-", dir="/tmp")))
        try:
            address = directory / "s"
            with Listener(peer, address):
                result = subprocess.run([str(harness), "launch", str(address), str(secret), str(scenario)], capture_output=True, text=True, env={**os.environ, **(environment or {})}, timeout=30)
        finally:
            shutil.rmtree(directory, ignore_errors=True)
        check(f"launch {name}", result.stdout.strip(), expected, failures)
        return scenario

    refused = "launched" if lab else "refused"
    launch("trusted", trusted, "bridge", trusted, "launched")
    launch("adhoc-peer", "adhoc", "bridge", trusted, refused)
    launch("other-certificate-peer", other, "bridge", trusted, refused)
    launch("other-identifier-peer", trusted, "other", trusted, refused)
    launch("peer-without-hardened-runtime", trusted, "bridge", trusted, refused, peer_hardened=False)
    launch("adhoc-session", trusted, "bridge", "adhoc", refused)
    launch("other-certificate-session", trusted, "bridge", other, refused)
    launch("other-identifier-session", trusted, "bridge", trusted, refused, session_identifier="other")

    override = work / "override-session"
    marker = work / "override-marker"
    override.write_text(f"#!/bin/sh\ntouch {marker}\nexec sleep 5\n")
    override.chmod(0o755)
    launch("session-override", trusted, "bridge", trusted, "launched-without-marker" if lab else "launched", environment={"AB_SESSION_BINARY": str(override)})
    check("session override ran", marker.exists(), lab, failures)

    install(bridge, trusted, "bridge")
    check("connector trusted", run(str(harness), "connector").stdout.strip(), "ran", failures)
    connector_marker = work / "connector-marker"
    connector_override = work / "connector-override"
    connector_override.write_text(f"#!/bin/sh\ntouch {connector_marker}\necho '[]'\n")
    connector_override.chmod(0o755)
    check("connector override", run(str(harness), "connector", str(connector_override)).stdout.strip(), "ran", failures)
    check("connector override ran", connector_marker.exists(), lab, failures)
    for name, identity in (("adhoc", "adhoc"), ("other certificate", other)):
        install(bridge, identity, "bridge")
        check(f"connector {name}", run(str(harness), "connector").stdout.strip(), "ran" if lab else "refused", failures)

    for name, identity, identifier, expected in (("trusted", trusted, "bridge", "admitted"), ("adhoc", "adhoc", "bridge", "admitted" if lab else "refused"), ("other certificate", other, "bridge", "admitted" if lab else "refused")):
        spawned = work / f"spawned-{name.replace(' ', '-')}"
        install(spawned, identity, identifier)
        spawned_marker = work / f"{spawned.name}-marker"
        check(f"running child {name}", run(str(harness), "admit", str(spawned), str(spawned_marker)).stdout.strip(), expected, failures)
        check(f"running child {name} ran", spawned_marker.exists(), expected == "admitted", failures)


def media_parents(fixture, binary, signing, trusted, other, work, failures):
    request = '{"kind":"mime","uti":"public.jpeg"}'
    for name, identity, identifier, expected in (("trusted session", trusted, "ab-session", 0), ("other certificate", other, "ab-session", 1), ("other identifier", trusted, "bridge", 1)):
        parent = work / f"parent-{name.replace(' ', '-')}"
        shutil.copy2(fixture, parent)
        signing.sign(parent, identity, identifier)
        result = subprocess.run([str(parent), "run", str(binary), "media-facts"], input=request, capture_output=True, text=True, timeout=30)
        check(f"media facts from {name}", result.returncode, expected, failures)


def helper_attacks(helper, signing, trusted, other, fixture, failures):
    work = private_directory(signing.work / "helper")
    app = work / "AppleBridgeHelper.app"
    macos = application(app, helper, "AppleBridgeHelper")
    binary = macos / "AppleBridgeHelper"
    signing.sign(binary, trusted, "dev.applebridge.identity-test")
    marker = work / "marker"
    child = work / "attacker-child"
    child.write_text(f"#!/bin/sh\ntouch {marker}\necho '[]'\nexec sleep 3\n")
    child.chmod(0o755)
    home = private_directory(work / "home")
    secret = work / "secret"
    secret.write_text("synthetic")
    secret.chmod(0o600)

    check("override text absent from release Helper", b"AB_SESSION_BINARY" in binary.read_bytes(), False, failures)

    facts = subprocess.run([str(binary), "media-facts"], input='{"kind":"mime","uti":"public.jpeg"}', capture_output=True, text=True, timeout=30)
    check("media facts refused for an unverified parent", facts.returncode, 1, failures)
    if fixture:
        media_parents(fixture, binary, signing, trusted, other, work, failures)

    directory = private_directory(Path(tempfile.mkdtemp(prefix="ab-id-", dir="/tmp")))
    server = socket.socket(socket.AF_UNIX)
    address = directory / "s"
    try:
        server.bind(str(address))
        address.chmod(0o600)
        server.listen(8)
        process = subprocess.Popen([str(binary), "session", "--socket", str(address), "--secret-file", str(secret), "--home", str(home), "--gateway-pid", str(os.getpid())], env={**os.environ, "AB_SESSION_BINARY": str(child)}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(8)
        ran = marker.exists()
        process.send_signal(signal.SIGKILL)
        process.wait()
    finally:
        server.close()
        shutil.rmtree(directory, ignore_errors=True)
    check("session override child ran against an attacker socket", ran, False, failures)
    marker.unlink(missing_ok=True)

    process = subprocess.Popen([str(binary), "connector-approvals", str(home), str(child)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(8)
    ran = marker.exists()
    process.send_signal(signal.SIGKILL)
    process.wait()
    check("connector override child ran", ran, False, failures)


def check(name, actual, expected, failures):
    passed = actual == expected
    print(f"{'PASS' if passed else 'FAIL'} {name}: {actual!r}" + ("" if passed else f" (expected {expected!r})"))
    if not passed:
        failures.append(name)


def main():
    parser = argparse.ArgumentParser(description="Synthetic Helper code-identity checks. Uses throwaway test certificates and no real permissions.")
    parser.add_argument("--output", type=Path, help="native build output holding HelperIdentityChecks and HelperIdentityFixture")
    parser.add_argument("--lab", action="store_true", help="the output is a LAB build, where overrides stay available")
    parser.add_argument("--helper", type=Path, help="a release AppleBridgeHelper binary to attack on a test-signed copy")
    arguments = parser.parse_args()
    failures = []
    work = Path(tempfile.mkdtemp(prefix="ab-helper-identity-"))
    signing = Signing(work)
    try:
        trusted = signing.identity("Apple Bridge Identity Test A")
        other = signing.identity("Apple Bridge Identity Test B")
        if arguments.output:
            synthetic_checks(arguments.output, signing, trusted, other, arguments.lab, failures)
        if arguments.helper:
            fixture = arguments.output / "HelperIdentityFixture" if arguments.output else None
            helper_attacks(arguments.helper, signing, trusted, other, fixture, failures)
    finally:
        signing.close()
        shutil.rmtree(work, ignore_errors=True)
    print(f"{len(failures)} failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
