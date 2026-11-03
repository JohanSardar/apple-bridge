import argparse
import json
from pathlib import Path
import subprocess

HARNESS_ONLY = {'credential_backend_override_restored'}
SCENARIOS = ['CORE-QUERY-ROUNDTRIP', 'CORE-QUERY-BLOB']


def verdicts(path):
    rows = {}
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            row = json.loads(line)
            rows[row['scenario']] = {a['name']: a['passed'] for a in row['assertions']
                                     if a['name'] not in HARNESS_ONLY}
    return rows


def main():
    parser = argparse.ArgumentParser()
    for name in ['families', 'go-lab', 'gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--scenario', action='append')
    args = parser.parse_args()
    out = Path(args.output).resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2] / 'lab' / 'scenarios' / 'core'
    files = [str(root / f'{name}.yaml') for name in args.scenario or SCENARIOS]
    with (out / 'go.jsonl').open('w') as stream:
        subprocess.run([args.go_lab, 'run', *files, '--level', 'sim', '--json'],
                       stdout=stream, timeout=900, check=True)
    with (out / 'rust.jsonl').open('w') as stream:
        subprocess.run([args.families, 'run', *files, '--gateway', args.gateway,
                        '--simapple', args.simapple, '--output', str(out / 'worlds')],
                       stdout=stream, timeout=900, check=True)
    go, rust = verdicts(out / 'go.jsonl'), verdicts(out / 'rust.jsonl')
    if len(go) != len(files) or go != rust or not all(all(v.values()) for v in rust.values()):
        raise SystemExit(f'verdict mismatch\ngo={go}\nrust={rust}')
    print(json.dumps({'scenarios': sorted(rust), 'identical': True}))


if __name__ == '__main__':
    main()
