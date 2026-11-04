import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('sim_parallel.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def make_executable(path, body):
    path.write_text('#!/bin/sh\n' + body + '\n')
    path.chmod(0o700)


class RunnerTests(unittest.TestCase):
    def check_command(self, command, timeout=5):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            families = root / 'families'
            make_executable(families, command)
            gateway = root / 'bridge'
            gateway.write_text('')
            with patch.object(runner.platform, 'system', return_value='Linux'):
                return runner.execute({'id': 'fixture', 'path': 'fixture.yaml', 'packages': []}, timeout, root, str(gateway), '', str(families), '/sim/apple')

    def test_startup_blockers_prevent_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / 'executed'
            families = root / 'families'
            make_executable(families, 'touch "' + str(marker) + '"')
            entry = {'id': 'blocked', 'path': 'blocked.yaml', 'packages': ['ab-runtime']}
            for startup, gateway, expected in [('ab-runtime: gateway ipc/1 socket unavailable', '/usr/bin/true', 'ab-runtime: gateway ipc/1 socket unavailable'), ('', str(root / 'missing'), 'ab-runtime: Rust bridge executable unavailable')]:
                with patch.object(runner.platform, 'system', return_value='Linux'):
                    row = runner.execute(entry, 5, root, gateway, startup, str(families), '/sim/apple')
                self.assertFalse(marker.exists(), 'blocked scenario process was started')
                self.assertEqual(row['status'], 'blocked')
                self.assertEqual(row['execution_status'], 'not-started')
                self.assertEqual(row['blocked_on'], [expected])
            with patch.object(runner.platform, 'system', return_value='Linux'):
                row = runner.execute(entry, 5, root, '/usr/bin/true', '', str(families), '/sim/apple')
            self.assertTrue(marker.exists())
            self.assertEqual(row['status'], 'pass')
            self.assertEqual(row['blocked_on'], [])

    def test_no_runner_scenarios_report_without_running(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / 'executed'
            families = root / 'families'
            make_executable(families, 'touch "' + str(marker) + '"')
            row = runner.execute({'id': 'AGENT-SUITE', 'path': 'a.yaml', 'packages': []}, 5, root, '/usr/bin/true', '', str(families), '/sim/apple', {'AGENT-SUITE': 'no agent runner'})
            self.assertFalse(marker.exists())
            self.assertEqual(row['status'], 'no-runner')
            self.assertEqual(row['reason'], 'no agent runner')

    def test_no_runner_table_names_the_four_scenarios(self):
        self.assertEqual(sorted(runner.load_no_runner()), ['AGENT-SUITE', 'QUERY-AGENT', 'RMT-ONECOMMAND-FLOW', 'RMT-TS-EMBEDDED'])

    def test_native_start_handshake(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bin').mkdir()
            make_executable(root / 'bin/ab-local', 'exec "$@"')
            families = root / 'families'
            make_executable(families, 'exit 0')
            with patch.object(runner.platform, 'system', return_value='Darwin'), patch.object(runner.Path, 'home', return_value=root):
                row = runner.execute({'id': 'native', 'path': 'fixture.yaml', 'packages': []}, 5, root, '/usr/bin/true', '', str(families), '/sim/apple')
            self.assertEqual(row['status'], 'pass')
            self.assertTrue((root / 'native.started').exists())

    def test_runs_families_with_gateway_and_simapple(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            families = root / 'families'
            make_executable(families, 'printf "%s\\n" "$@"')
            gateway = root / 'bridge'
            gateway.write_text('')
            scenario = root / 'fixture.yaml'
            scenario.write_text('id: fixture\n')
            with patch.object(runner.platform, 'system', return_value='Linux'):
                row = runner.execute({'id': 'fixture', 'path': str(scenario), 'packages': []}, 5, root, str(gateway), '', str(families), '/sim/apple')
            self.assertEqual((root / 'fixture.log').read_text().splitlines(), ['run', str(scenario), '--gateway', str(gateway), '--simapple', '/sim/apple', '--output', str(root / 'fixture.world')])
            self.assertEqual(row['status'], 'pass')

    def test_default_and_end_sets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            families = root / 'families'
            make_executable(families, 'exit 0')
            simapple = root / 'simapple'
            make_executable(simapple, 'exit 0')
            gateway = root / 'bridge'
            make_executable(gateway, 'while [ $# -gt 0 ]; do [ "$1" = --socket ] && socket=$2; shift; done\ntouch "$socket"\nexec sleep 30')
            inventory = root / 'map.json'
            inventory.write_text(json.dumps({'selection': {'slow': ['slow']}, 'scenarios': [
                {'id': 'fast', 'path': 'fast.yaml', 'levels': ['sim'], 'host': 'linux', 'recommendation': 'keep', 'packages': []},
                {'id': 'slow', 'path': 'slow.yaml', 'levels': ['sim'], 'host': 'linux', 'recommendation': 'run-at-end', 'packages': []},
            ]}))
            for index, (flags, expected) in enumerate([([], 'fast'), (['--run-at-end'], 'slow'), (['--select', 'slow'], 'slow')]):
                output = root / str(index)
                argv = ['runner', '--rust-binary', str(gateway), '--families-binary', str(families), '--simapple-binary', str(simapple), '--map', str(inventory), '--output', str(output), *flags]
                with patch.object(sys, 'argv', argv), patch.object(runner.platform, 'system', return_value='Linux'):
                    self.assertEqual(runner.main(), 0)
                report = json.loads((output / 'summary.json').read_text())
                self.assertEqual([r['id'] for r in report['results']], [expected])

    def test_registered_end_gates(self):
        inventory = json.loads((Path(__file__).parents[1] / 'test-selection.json').read_text())
        entries = {e['id']: e for e in inventory['scenarios']}
        for identity in ['MAIL-IDLE', 'SAF-IDLE', 'CALL-IDLE', 'RELAY-STREAMS', 'MSG-WATCHER-REARM', 'CAL-STORE-READ', 'WEB-RM-THREAD', 'WEB-RM-CHATS', 'WEB-RM-REMINDERS-ITEMS']:
            self.assertEqual(entries[identity]['recommendation'], 'run-at-end', identity)

    def test_success(self):
        self.assertEqual(self.check_command('exit 0')['status'], 'pass')

    def test_failure(self):
        self.assertEqual(self.check_command('exit 7')['status'], 'fail')

    def test_hard_timeout(self):
        row = self.check_command('sleep 60 &\nwait', .05)
        self.assertEqual(row['status'], 'timeout')
        self.assertLess(row['seconds'], 12)


if __name__ == '__main__':
    unittest.main()
