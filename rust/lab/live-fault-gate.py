import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

RUST_GATES = {
    'skip-checkpoint-commit': ('LIVE-CHECKPOINT-REOPEN', 'reopen_offers_committed_checkpoint'),
    'accept-stale-fence': ('LIVE-STALE-FENCE', 'stale_fence_rejected'),
    'ack-uncommitted-data': ('LIVE-ACK-DURABLE', 'acked_data_visible'),
    'leak-scope': ('LIVE-SCOPE-QUERY', 'scoped_token_hides_out_of_scope'),
}


def environment_for(fault):
    environment = dict(os.environ)
    environment.pop('AB_LAB_SEEDED_FAULT', None)
    if fault:
        environment['AB_LAB_SEEDED_FAULT'] = fault
    return environment


def parse_records(stdout):
    return [json.loads(line) for line in stdout.splitlines() if line.strip().startswith('{')]


def run_rust(runner, scenario, gateway, simapple, output, fault):
    result = subprocess.run([runner, 'run', str(scenario), '--gateway', gateway, '--simapple', simapple,
                             '--output', str(output)], capture_output=True, text=True, timeout=300,
                            env=environment_for(fault), check=False)
    return result.returncode, parse_records(result.stdout)


def rust_healthy(code, records, name):
    return code == 0 and len(records) == 1 and records[0]['passed'] and any(
        a['name'] == name and a['passed'] for a in records[0]['assertions'])


def rust_detected(code, records, name):
    if code == 0 or len(records) != 1 or records[0]['passed']:
        return False
    assertions = records[0]['assertions']
    infrastructure = {'action', 'cleanup', 'sim_level', 'assertions_present'}
    if any(not a['passed'] and a['name'] in infrastructure for a in assertions):
        return False
    return any(a['name'] == name and not a['passed'] for a in assertions)


def main():
    parser = argparse.ArgumentParser()
    for name in ['runner', 'healthy-gateway', 'mutant-gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    destination = Path(args.output).resolve()
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    if len(str(destination)) > 48:
        raise SystemExit('output path too long for unix sockets: ' + str(destination))
    root = Path(__file__).resolve().parent / 'scenarios'
    verdicts = {}
    for number, (fault, (scenario, assertion)) in enumerate(RUST_GATES.items()):
        path = root / (scenario + '.yaml')
        code, records = run_rust(args.runner, path, args.healthy_gateway, args.simapple,
                                 destination / ('h' + str(number)), '')
        healthy = rust_healthy(code, records, assertion)
        code, records = run_rust(args.runner, path, args.mutant_gateway, args.simapple,
                                 destination / ('m' + str(number)), fault)
        verdicts[fault] = {'scenario': scenario, 'assertion': assertion, 'healthy_passes': healthy,
                           'mutant_detected': rust_detected(code, records, assertion)}
    hashes = {name: hashlib.sha256(Path(getattr(args, name.replace('-', '_'))).read_bytes()).hexdigest()
              for name in ['runner', 'healthy-gateway', 'mutant-gateway', 'simapple']}
    summary = {'binary_sha256': hashes, 'rust_runner': verdicts}
    (destination / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    rust_ok = all(v['healthy_passes'] and v['mutant_detected'] for v in verdicts.values())
    if not rust_ok:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
