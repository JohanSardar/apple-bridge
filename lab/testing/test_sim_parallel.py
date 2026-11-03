import importlib.util
from pathlib import Path
import tempfile
from unittest.mock import patch
import unittest

spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('sim_parallel.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def check_command(self, command, timeout=5):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'lab'
            binary.write_text('#!/bin/sh\n' + command + '\n')
            binary.chmod(0o700)
            with patch.object(runner.platform, 'system', return_value='Linux'):
                return runner.execute({'id': 'fixture', 'path': 'fixture.yaml'}, str(binary), timeout, root)

    def test_native_start_handshake(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bin').mkdir()
            wrapper = root / 'bin/ab-local'
            wrapper.write_text('#!/bin/sh\nexec \"$@\"\n')
            wrapper.chmod(0o700)
            with patch.object(runner.platform, 'system', return_value='Darwin'), patch.object(runner.Path, 'home', return_value=root):
                row = runner.execute({'id': 'native', 'path': 'fixture.yaml'}, '/usr/bin/true', 5, root)
            self.assertEqual(row['status'], 'pass')
            self.assertTrue((root / 'native.started').exists())

    def test_always_runs_sim_level(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'lab'
            binary.write_text('#!/bin/sh\nprintf \"%s\\n\" \"$@\"\n')
            binary.chmod(0o700)
            with patch.object(runner.platform, 'system', return_value='Linux'):
                runner.execute({'id': 'fixture', 'path': 'fixture.yaml'}, str(binary), 5, root)
            self.assertEqual((root / 'fixture.log').read_text().splitlines(), ['run', 'fixture.yaml', '--level', 'sim', '--prebuilt', '--json'])

    def test_default_and_end_sets(self):
        import json
        import sys
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'lab'
            binary.write_text('#!/bin/sh\nexit 0\n')
            binary.chmod(0o700)
            inventory = root / 'map.json'
            inventory.write_text(json.dumps({'selection': {'slow': ['slow']}, 'scenarios': [
                {'id': 'fast', 'path': 'fast.yaml', 'levels': ['sim'], 'host': 'linux', 'recommendation': 'keep'},
                {'id': 'slow', 'path': 'slow.yaml', 'levels': ['sim'], 'host': 'linux', 'recommendation': 'run-at-end'},
            ]}))
            for index, (flags, expected) in enumerate([([], 'fast'), (['--run-at-end'], 'slow'), (['--select', 'slow'], 'slow')]):
                output = root / str(index)
                argv = ['runner', '--binary', str(binary), '--map', str(inventory), '--output', str(output), *flags]
                with patch.object(sys, 'argv', argv), patch.object(runner.platform, 'system', return_value='Linux'):
                    self.assertEqual(runner.main(), 0)
                report = json.loads((output / 'summary.json').read_text())
                self.assertEqual([r['id'] for r in report['results']], [expected])

    def test_registered_end_gates(self):
        import json
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
