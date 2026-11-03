import argparse
import json
from pathlib import Path
import subprocess

HARNESS_ONLY = {'credential_backend_override_restored'}
BLOB_SCENARIOS = ['CORE-BLOB-CAP', 'CORE-BLOB-CORRUPT', 'CORE-BLOB-DUPLICATE-REFERENCE',
                  'CORE-BLOB-GATEWAY-RETRY', 'CORE-BLOB-GC', 'CORE-BLOB-INTERRUPT',
                  'CORE-BLOB-NO-DIGEST-AUTH', 'CORE-BLOB-OFFLOADED', 'CORE-BLOB-REFERENCE-INDEX',
                  'CORE-BLOB-SLOW-READ', 'CORE-BLOB-STAGE-DURING-TRANSFER',
                  'CORE-BLOB-STAGE-SAME-DIGEST']
JOURNEY_SCENARIOS = ['CORE-CMD-IDEMPOTENT', 'CORE-CMD-ERA', 'CORE-CMD-LOST-RESPONSE', 'CORE-CMD-CONFLICT',
                     'CORE-CMD-READ-YOUR-WRITE', 'CORE-CMD-CRASH-AMBIGUOUS', 'CORE-CMD-EXPIRY',
                     'CORE-CMD-CLOCK-JUMP', 'CORE-CMD-OFFLINE', *BLOB_SCENARIOS]
JOURNEY_RUST_ONLY = {'world_started'}
IMPORT_SCENARIOS = ['messages/MSG-HISTORY-IMPORT', 'contacts/CON-IMPORT', 'notes/NOTE-IMPORT', 'usage/USE-IMPORT']
MEDIA_SCENARIOS = ['web/WEB-MEDIA-VARIANTS', 'messages/MSG-MEDIA-CLEANUP']
WEB_RM_SCENARIOS = ['web/WEB-RM-MEDIA-COLLECTION', 'web/WEB-RM-SEARCH']
SCENARIOS = ['CORE-QUERY-ROUNDTRIP', 'CORE-QUERY-BLOB', 'CORE-QUERY-SCOPE', 'CORE-QUERY-DEADLINE',
             *JOURNEY_SCENARIOS]


def ignored(scenario, name):
    if name in HARNESS_ONLY or name == 'world_started':
        return True
    return scenario in JOURNEY_SCENARIOS and name in JOURNEY_RUST_ONLY


def verdicts(path):
    rows = {}
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            row = json.loads(line)
            rows[row['scenario']] = {a['name']: (a['passed'], a.get('status', ''))
                                     for a in row['assertions']
                                     if not ignored(row['scenario'], a['name'])}
    return rows


def main():
    parser = argparse.ArgumentParser()
    for name in ['families', 'go-lab', 'gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--scenario', action='append')
    parser.add_argument('--import', dest='imports', action='store_true')
    parser.add_argument('--media', dest='media', action='store_true')
    parser.add_argument('--web-rm', dest='web_rm', action='store_true')
    args = parser.parse_args()
    if args.imports:
        args.scenario = (args.scenario or []) + IMPORT_SCENARIOS
    if args.media:
        args.scenario = (args.scenario or []) + MEDIA_SCENARIOS
    if args.web_rm:
        args.scenario = (args.scenario or []) + WEB_RM_SCENARIOS
    out = Path(args.output).resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2] / 'lab' / 'scenarios'
    names = [n if '/' in n else f'core/{n}' for n in args.scenario or SCENARIOS]
    files = [str(root / f'{name}.yaml') for name in names]
    with (out / 'go.jsonl').open('w') as stream:
        subprocess.run([args.go_lab, 'run', *files, '--level', 'sim', '--json'],
                       stdout=stream, timeout=900, check=True)
    with (out / 'rust.jsonl').open('w') as stream:
        subprocess.run([args.families, 'run', *files, '--gateway', args.gateway,
                        '--simapple', args.simapple, '--output', str(out / 'worlds')],
                       stdout=stream, timeout=900, check=True)
    go, rust = verdicts(out / 'go.jsonl'), verdicts(out / 'rust.jsonl')
    if len(go) != len(files) or go != rust or not all(all(p or st == 'unverified' for p, st in v.values()) for v in rust.values()):
        raise SystemExit(f'verdict mismatch\ngo={go}\nrust={rust}')
    print(json.dumps({'scenarios': sorted(rust), 'identical': True}))


if __name__ == '__main__':
    main()
