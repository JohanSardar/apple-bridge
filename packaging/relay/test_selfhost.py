#!/usr/bin/env python3
import hashlib
import os
import pathlib
import re
import stat
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SELFHOST = ROOT / "deploy" / "selfhost"
INSTALL = SELFHOST / "install.sh"


def unit(name):
    fields = {}
    for line in (SELFHOST / "systemd" / f"{name}.service").read_text().splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            fields.setdefault(key, value)
    return fields


class Identities(unittest.TestCase):
    def test_each_service_runs_as_its_own_user(self):
        users = {name: unit(name)["User"] for name in ("ab-relay", "ab-relay-control", "ab-accounts")}
        self.assertEqual(
            users,
            {"ab-relay": "ab-relay", "ab-relay-control": "ab-control", "ab-accounts": "ab-accounts"},
        )
        for name, user in users.items():
            self.assertEqual(unit(name)["Group"], user)

    def test_state_directories_are_private_to_their_service(self):
        for name in ("ab-relay", "ab-relay-control", "ab-accounts"):
            self.assertEqual(unit(name)["StateDirectoryMode"], "0750")

    def test_only_the_service_that_needs_a_secret_can_read_it(self):
        script = INSTALL.read_text()
        self.assertIn('chown root:ab-control "$token_file"', script)
        self.assertIn('chown root:ab-accounts "$secret"', script)
        self.assertIn("chown root:root /etc/ab-relay/relay.env", script)
        self.assertIn("chown root:root /etc/ab-relay/accounts.env /etc/ab-relay/control.env", script)
        self.assertNotRegex(script, r"root:ab-relay[^\n]*(token_file|secret|\.env)")

    def test_an_upgrade_moves_existing_state_to_the_new_owners(self):
        script = INSTALL.read_text()
        self.assertIn("own_state ab-control /var/lib/ab-relay-control", script)
        self.assertIn("own_state ab-accounts /var/lib/ab-accounts", script)
        self.assertLess(
            script.index("systemctl stop ab-relay-control ab-accounts"),
            script.index("own_state ab-control"),
            "state must not change owner under a running service",
        )

    def test_nothing_running_is_touched_until_every_file_is_ready(self):
        script = INSTALL.read_text()
        stop = script.index("systemctl stop ab-relay-control ab-accounts")
        for written in (
            ">/etc/ab-relay/relay.env",
            "cat >/etc/ab-relay/accounts.env",
            "cat >/etc/ab-relay/control.env",
            "/usr/local/lib/ab-relay/accounts.mjs",
        ):
            self.assertLess(script.index(written), stop, written)
        for cutover in (
            'chown root:ab-control "$token_file"',
            'chown root:ab-accounts "$secret"',
            '/etc/systemd/system/ab-relay.service',
            '"$source_dir"/systemd/ab-accounts.service',
            'ab-relay-admin" /usr/local/bin/ab-relay-admin',
            "systemctl daemon-reload",
        ):
            self.assertGreater(script.index(cutover), stop, cutover)

    def test_a_failed_install_says_how_to_recover(self):
        self.assertIn("run it again", INSTALL.read_text())

    def test_the_admin_tool_reaches_accounts_as_the_accounts_user(self):
        self.assertIn("runuser -u ab-accounts", (SELFHOST / "ab-relay-admin").read_text())


class NodeRuntime(unittest.TestCase):
    def fetch(self, served, pinned=None):
        with tempfile.TemporaryDirectory() as directory:
            work = pathlib.Path(directory)
            stub = work / "curl"
            stub.write_text(
                "#!/bin/sh\n"
                'while [ $# -gt 0 ]; do\n'
                '  case "$1" in\n'
                '    -o) shift; target=$1 ;;\n'
                "  esac\n"
                "  shift\n"
                "done\n"
                f'cat "{work}/served" > "${{target:-/dev/stdout}}"\n'
            )
            stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
            (work / "served").write_bytes(served)
            override = f"NODE_SHA256={pinned}; " if pinned else ""
            return subprocess.run(
                [
                    "bash",
                    "-c",
                    'set -euo pipefail; AB_INSTALL_SOURCE_ONLY=1; . "$0"; '
                    + override
                    + 'fetch_node_archive "$1/archive"',
                    str(INSTALL),
                    str(work),
                ],
                env={**os.environ, "PATH": f"{work}:{os.environ['PATH']}"},
                capture_output=True,
                text=True,
            )

    def test_an_altered_archive_is_refused_even_when_the_download_site_vouches_for_it(self):
        result = self.fetch(b"not the real node")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stderr)

    def test_the_pinned_archive_is_accepted(self):
        archive = b"a release"
        result = self.fetch(archive, hashlib.sha256(archive).hexdigest())
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_the_checksum_is_pinned_and_never_fetched(self):
        script = INSTALL.read_text()
        self.assertRegex(script, r"(?m)^NODE_SHA256=[0-9a-f]{64}$")
        self.assertNotIn("SHASUMS256", script)
        self.assertEqual(len(re.findall(r"^NODE_VERSION=", script, re.M)), 1)


if __name__ == "__main__":
    unittest.main()
