import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time


def execute(entry, binary, timeout, logs):
    started = time.monotonic()
    target = logs / (entry['id'] + '.log')
    with target.open('wb') as output:
        command = [binary, 'run', entry['path'], '--level', 'sim', '--prebuilt', '--json']
        marker = logs / (entry['id'] + '.started')
        if platform.system() == 'Darwin':
            handshake = 'import os,sys,time; open(sys.argv[1]+".tmp","w").write(str(time.monotonic())); os.replace(sys.argv[1]+".tmp",sys.argv[1]); os.execv(sys.argv[2],sys.argv[2:])'
            command = [str(Path.home() / 'bin/ab-local'), sys.executable, '-c', handshake, str(marker), *command]
        process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        status = 'fail'
        try:
            if platform.system() == 'Darwin':
                queue_deadline = time.monotonic() + 3600
                while not marker.exists() and process.poll() is None:
                    if time.monotonic() >= queue_deadline:
                        raise subprocess.TimeoutExpired(command, 3600)
                    time.sleep(.1)
                if marker.exists():
                    started = float(marker.read_text())
            code = process.wait(timeout=max(.01, started + timeout - time.monotonic()))
            if code == 0:
                status = 'pass'
        except subprocess.TimeoutExpired:
            status = 'timeout'
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    return {'id': entry['id'], 'seconds': round(time.monotonic() - started, 3), 'status': status}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--parallel', type=int, default=1 if platform.system() == 'Darwin' else 4)
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--select', action='append', default=[])
    parser.add_argument('--run-at-end', action='store_true')
    parser.add_argument('--host', choices=['linux', 'darwin', 'all'], default=platform.system().lower())
    parser.add_argument('--binary', default='bin/bridge-lab')
    parser.add_argument('--map', default='lab/test-selection.json')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    if args.parallel < 1 or args.timeout <= 0:
        parser.error('parallel and timeout must be positive')
    if platform.system() == 'Darwin' and args.parallel != 1:
        parser.error('Air scenarios must run serially with --parallel 1 through ab-local')
    inventory = json.loads(Path(args.map).read_text())
    known = inventory['selection']
    ids = set()
    for key in args.select:
        if key not in known:
            parser.error('unknown selection: ' + key)
        ids.update(known[key])
    entries = [e for e in inventory['scenarios'] if 'sim' in e['levels'] and (not args.select or e['id'] in ids) and (args.host == 'all' or e['host'] == args.host) and (bool(args.select) or (e['recommendation'] == 'run-at-end') == args.run_at_end)]
    if not entries:
        parser.error('selection contains no SIM scenarios for this host')
    binary_path = Path(args.binary).resolve()
    if not binary_path.is_file() or not os.access(binary_path, os.X_OK):
        parser.error('prebuilt bridge-lab executable required')
    destination = Path(args.output)
    if destination.exists():
        parser.error('output already exists; use a new run directory')
    destination.mkdir(parents=True, mode=0o700)
    binary = str(binary_path)
    started = time.monotonic()
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel) as pool:
        futures = [pool.submit(execute, e, binary, args.timeout, destination) for e in entries]
        for future in concurrent.futures.as_completed(futures):
            row = future.result()
            rows.append(row)
            print(f"{row['id']:<45} {row['status']:<9} {row['seconds']:>9.3f}s", flush=True)
            report = {'wall_seconds': round(time.monotonic() - started, 3), 'parallel': args.parallel, 'host': platform.system().lower(), 'complete': len(rows) == len(entries), 'expected': len(entries), 'results': sorted(rows, key=lambda r: r['id'])}
            temporary = destination / 'summary.json.tmp'
            temporary.write_text(json.dumps(report, indent=2) + '\n')
            temporary.replace(destination / 'summary.json')
    print(f"Wall: {report['wall_seconds']:.3f}s; {len(rows)} scenarios", flush=True)
    return int(any(r['status'] != 'pass' for r in rows))


if __name__ == '__main__':
    sys.exit(main())
