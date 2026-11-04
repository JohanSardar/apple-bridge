import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import sys

from shadow import private_directory


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--fixture', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--left', '--go', dest='left', required=True)
    parser.add_argument('--right', '--rust', dest='right', required=True)
    args = parser.parse_args()
    fixture = Path(args.fixture).resolve()
    output = private_directory(args.output)
    if list(output.iterdir()):
        raise ValueError('decision proof output must be empty')
    with sqlite3.connect((fixture / 'home/state.db').as_uri() + '?mode=ro', uri=True) as db:
        aliases = {'\0'.join(row[:4]): row[4] for row in db.execute('SELECT source,generation,collection,native_key,record_id FROM aliases')}
    with sqlite3.connect((fixture / 'home/ledger.db').as_uri() + '?mode=ro', uri=True) as db:
        now = json.loads(db.execute('SELECT payload FROM commands ORDER BY rowid LIMIT 1').fetchone()[0])['created_at']
    decisions = output / 'decisions.json'
    decisions.write_text(json.dumps({'now': now, 'aliases': aliases}))
    decisions.chmod(0o600)
    result = subprocess.run([sys.executable, str(Path(__file__).with_name('shadow.py')), 'replay', '--recording', str(fixture / 'recording/ipc.shadow'), '--starting-home', str(fixture / 'starting'), '--output', str(output / 'replayed'), '--left', args.left, '--right', args.right, '--decisions', str(decisions), '--exact'], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError('decision replay failed; private diagnostic retained')
    report = json.loads(result.stdout)
    count = sum(not row['equal'] for row in report['digests'])
    if count:
        raise RuntimeError('exact decision replay differs')
    print(json.dumps({'exact': True, 'differing_tables': count, 'fixed_clock_fixture_only': True}))


if __name__ == '__main__':
    main()
