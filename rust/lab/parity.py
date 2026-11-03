import argparse
import json
import os
from pathlib import Path
import subprocess

SCENARIOS = [
    'core/CORE-CMD-IDEMPOTENT.yaml',
    'core/CORE-CMD-CRASH-AMBIGUOUS.yaml',
    'core/CORE-KILL-GATEWAY-CAPTURE.yaml',
    'messages/MSG-SEND-IDENTICAL-MANUAL.yaml',
    'messages/MSG-UI-INTERFERENCE.yaml',
    'reminders/REM-PERMISSION-LOSS.yaml',
]


def run(args):
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = str(Path(args.rust_peer).resolve(strict=True))
    lab = str(Path(args.go_lab).resolve(strict=True))
    scenarios = args.scenario or SCENARIOS
    expected = {}
    summary = []
    failures = []
    for repetition in range(1, args.repetitions + 1):
        for implementation in ['go', 'rust']:
            env = os.environ.copy()
            env.pop('AB_LAB_SIMAPPLE', None)
            if implementation == 'rust':
                env['AB_LAB_SIMAPPLE'] = binary
            command = [lab, 'run', *['lab/scenarios/' + s for s in scenarios], '--level', 'sim', '--local-replicas', '--json']
            destination = output / f'{implementation}-{repetition}.jsonl'
            print(f'parity {implementation} run {repetition}', flush=True)
            with destination.open('w') as stream:
                result = subprocess.run(command, env=env, stdout=stream, timeout=600, check=False)
            evidence = [json.loads(line) for line in destination.read_text().splitlines()]
            assert len(evidence) == len(scenarios), (destination, 'incomplete suite')
            assert sorted(item['scenario'] for item in evidence) == sorted(Path(s).stem for s in scenarios), (destination, 'unexpected scenarios')
            for item in evidence:
                verdict = sorted((a['name'], a.get('passed', False), a.get('status', '')) for a in item['assertions'])
                key = item['scenario']
                if not item['passed']:
                    failures.append(f'{destination}: {key} failed')
                assert item['conditions']['simapple_impl'] == implementation
                if key in expected:
                    if expected[key] != verdict:
                        failures.append(f'{destination}: {key} different assertion verdicts')
                else:
                    expected[key] = verdict
                summary.append({'scenario': key, 'simapple': implementation, 'run': repetition, 'passed': item['passed']})
            if result.returncode != 0:
                failures.append(f'{destination}: exit {result.returncode}')
    (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    assert not failures, '\n'.join(failures)
    print(f'{len(summary)} scenario executions: identical assertion verdicts across both peers')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--go-lab', required=True)
    parser.add_argument('--rust-peer', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--scenario', action='append', help='path relative to lab/scenarios; repeat per scenario')
    parser.add_argument('--repetitions', type=int, choices=range(1, 4), default=1)
    run(parser.parse_args())
