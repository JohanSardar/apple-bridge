import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

GATES = {
    'skip-checkpoint-commit': ('LIVE-CHECKPOINT-REOPEN', 'reopen_offers_committed_checkpoint'),
    'ack-uncommitted-data': ('LIVE-ACK-DURABLE', 'acked_data_visible'),
}
INFRASTRUCTURE = {'action', 'cleanup', 'sim_level', 'assertions_present'}


def run_rust(runner, scenario, gateway, simapple, output, fault):
    environment = dict(os.environ)
    environment.pop('AB_LAB_SEEDED_FAULT', None)
    if fault:
        environment['AB_LAB_SEEDED_FAULT'] = fault
    result = subprocess.run([runner, 'run', str(scenario), '--gateway', gateway, '--simapple', simapple,
                             '--output', str(output)], capture_output=True, text=True, timeout=300,
                            env=environment, check=False)
    records = [json.loads(line) for line in result.stdout.splitlines() if line.strip().startswith('{')]
    return result.returncode, records


def run_go(go_lab, scenario, gateway, fault, output):
    environment = dict(os.environ)
    environment.pop('AB_LAB_SEEDED_FAULT', None)
    if fault:
        environment['AB_LAB_SEEDED_FAULT'] = fault
    result = subprocess.run([go_lab, 'run', str(scenario), '--level', 'sim', '--json', '--impl', 'gateway=rust',
                             '--rust-binary', gateway], capture_output=True, text=True, timeout=900,
                            env=environment, check=False, cwd=output)
    records = [json.loads(line) for line in result.stdout.splitlines() if line.strip().startswith('{')]
    return result.returncode, records


def healthy_ok(code, records, name):
    return code == 0 and len(records) == 1 and records[0]['passed'] and any(
        a['name'] == name and a['passed'] for a in records[0]['assertions'])


def detected(code, records, name):
    if code == 0 or len(records) != 1 or records[0]['passed']:
        return False
    assertions = records[0]['assertions']
    broken = [a for a in assertions if not a['passed']]
    return any(a['name'] == name for a in broken) and not any(a['name'] in INFRASTRUCTURE for a in broken)


def main():
    parser = argparse.ArgumentParser()
    for name in ['runner', 'healthy-gateway', 'mutant-gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--go-lab')
    parser.add_argument('--go-scenario', action='append', default=[])
    args = parser.parse_args()
    destination = Path(args.output).resolve()
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    if len(str(destination)) > 48:
        raise SystemExit('output path too long for unix sockets: ' + str(destination))
    root = Path(__file__).resolve().parent / 'scenarios'
    verdicts = {}
    for number, (fault, (scenario, assertion)) in enumerate(GATES.items()):
        path = root / (scenario + '.yaml')
        code, records = run_rust(args.runner, path, args.healthy_gateway, args.simapple,
                                 destination / ('h' + str(number)), '')
        healthy = healthy_ok(code, records, assertion)
        code, records = run_rust(args.runner, path, args.mutant_gateway, args.simapple,
                                 destination / ('m' + str(number)), fault)
        verdicts[fault] = {'scenario': scenario, 'assertion': assertion, 'rust_healthy_passes': healthy,
                           'rust_mutant_detected': detected(code, records, assertion)}
    go = {}
    if args.go_lab:
        work = destination / 'go'
        work.mkdir(mode=0o700)
        for scenario in args.go_scenario:
            path = Path(scenario).resolve()
            code, records = run_go(args.go_lab, path, args.healthy_gateway, '', work)
            healthy = code == 0 and len(records) == 1 and records[0]['passed']
            for fault in GATES:
                code, records = run_go(args.go_lab, path, args.mutant_gateway, fault, work)
                caught = code != 0 and len(records) == 1 and not records[0]['passed']
                go.setdefault(fault, []).append({'scenario': path.name, 'go_healthy_passes': healthy,
                                                 'go_mutant_detected': caught and healthy})
    hashes = {name: hashlib.sha256(Path(getattr(args, name.replace('-', '_'))).read_bytes()).hexdigest()
              for name in ['runner', 'healthy-gateway', 'mutant-gateway', 'simapple']}
    summary = {'binary_sha256': hashes, 'rust_runner': verdicts, 'go_harness': go}
    (destination / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    if not all(v['rust_healthy_passes'] and v['rust_mutant_detected'] for v in verdicts.values()):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
