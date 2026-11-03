import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def execute(command, destination):
    with destination.open('w') as stream:
        result = subprocess.run(command, stdout=stream, timeout=180, check=False)
    records = [json.loads(line) for line in destination.read_text().splitlines()]
    if result.returncode or not records or any(not row['passed'] for row in records):
        raise RuntimeError(f'failed execution: {destination}')
    return records


def main():
    parser = argparse.ArgumentParser()
    for name in ['runner', 'go-lab', 'gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    destination = Path(args.output).resolve()
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = Path(__file__).resolve().parent / 'scenarios'
    traces = sorted(str(path) for path in root.glob('LAB-TRACE-*.yaml'))
    smoke = sorted(str(path) for path in root.glob('RUST-CAPTURE-*.yaml'))
    rust = execute([args.runner, 'run', *traces, *smoke, '--gateway', args.gateway,
                    '--simapple', args.simapple, '--output', str(destination / 'worlds')],
                   destination / 'rust.jsonl')
    go = execute([args.go_lab, 'run', *traces, '--level', 'sim', '--json'], destination / 'go.jsonl')
    if len(rust) != len(traces) + len(smoke) or len(go) != len(traces):
        raise RuntimeError('incomplete execution')
    def faults(records):
        return {row['scenario']: sorted((a['name'], a['passed']) for a in row['assertions']
                                       if a['name'] in ['skip-checkpoint-commit', 'accept-stale-fence',
                                                        'ack-uncommitted-data', 'leak-scope'])
                for row in records if row['scenario'].startswith('LAB-TRACE-')}
    if faults(rust) != faults(go) or len(faults(go)) != 4:
        raise RuntimeError('trace detector parity failed')
    hashes = {name: hashlib.sha256(Path(getattr(args, name)).read_bytes()).hexdigest()
              for name in ['runner', 'gateway', 'simapple']}
    summary = {'binary_sha256': hashes, 'rust_scenarios': len(rust), 'go_trace_scenarios': len(go),
               'trace_detector_parity': True, 'live_gateway_mutants_verified': False,
               'scenarios': [{'scenario': row['scenario'], 'passed': row['passed']} for row in rust]}
    (destination / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
