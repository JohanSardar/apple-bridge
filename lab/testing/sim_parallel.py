import argparse
import concurrent.futures
import json
import os
import tempfile
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time


NO_RUNNER_FILE = Path(__file__).resolve().parents[1] / 'no-runner.json'


def load_no_runner():
    return json.loads(NO_RUNNER_FILE.read_text())


def execute(entry, timeout, logs, rust_binary, startup_blocker, families, simapple, no_runner=None):
    started = time.monotonic()
    target = logs / (entry['id'] + '.log')
    if entry['id'] in (no_runner or {}):
        reason = no_runner[entry['id']]
        target.write_text('no-runner: ' + reason + '\n')
        return {'id': entry['id'], 'seconds': 0, 'status': 'no-runner', 'reason': reason}
    blockers = []
    if startup_blocker:
        blockers.append(startup_blocker)
    if not rust_binary or not Path(rust_binary).is_file():
        blockers.append('ab-runtime: Rust bridge executable unavailable')
    if blockers:
        target.write_text('\n'.join(blockers) + '\n')
        return {'id': entry['id'], 'seconds': 0, 'status': 'blocked', 'execution_status': 'not-started', 'required_features': entry['packages'], 'blocked_on': blockers}
    with target.open('wb') as output:
        command = [families, 'run', entry['path'], '--gateway', rust_binary, '--simapple', simapple, '--output', str(logs / (entry['id'] + '.world'))]
        marker = logs / (entry['id'] + '.started')
        serialized = platform.system() == 'Darwin' and (Path.home() / 'bin/ab-local').is_file()
        if serialized:
            handshake = 'import os,sys,time; open(sys.argv[1]+".tmp","w").write(str(time.monotonic())); os.replace(sys.argv[1]+".tmp",sys.argv[1]); os.execv(sys.argv[2],sys.argv[2:])'
            command = [str(Path.home() / 'bin/ab-local'), sys.executable, '-c', handshake, str(marker), *command]
        process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        status = 'fail'
        try:
            if serialized:
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
    scenario_blockers = []
    if status == 'fail':
        for line in target.read_text(errors='replace').splitlines():
            if not line.startswith('{'):
                continue
            try:
                evidence = json.loads(line)
            except json.JSONDecodeError:
                continue
            if evidence.get('status') == 'blocked':
                status = 'blocked'
                scenario_blockers = [a.get('detail', 'harness: binary fixture adapter required') for a in evidence.get('assertions', []) if a.get('name') == 'binary_harness_required']
                if not scenario_blockers:
                    scenario_blockers = ['harness: scenario reported blocked']
    row = {'id': entry['id'], 'seconds': round(time.monotonic() - started, 3), 'status': status, 'required_features': entry['packages'], 'blocked_on': scenario_blockers}
    if scenario_blockers:
        row['execution_status'] = status
        row['status'] = 'blocked'
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--parallel', type=int, default=1 if platform.system() == 'Darwin' else 4)
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--select', action='append', default=[])
    parser.add_argument('--run-at-end', action='store_true')
    parser.add_argument('--host', choices=['linux', 'darwin', 'all'], default=platform.system().lower())
    parser.add_argument('--families-binary', required=True)
    parser.add_argument('--simapple-binary', required=True)
    parser.add_argument('--level', choices=['sim', 'replica'], default='sim')
    parser.add_argument('--rust-binary', required=True)
    parser.add_argument('--map', default='lab/test-selection.json')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    if args.parallel < 1 or args.timeout <= 0:
        parser.error('parallel and timeout must be positive')
    if platform.system() == 'Darwin' and (Path.home() / 'bin/ab-local').is_file() and args.parallel != 1:
        parser.error('Air scenarios must run serially with --parallel 1 through ab-local')
    inventory = json.loads(Path(args.map).read_text())
    known = inventory['selection']
    ids = set()
    for key in args.select:
        if key not in known:
            parser.error('unknown selection: ' + key)
        ids.update(known[key])
    for name in (args.families_binary, args.simapple_binary):
        if not Path(name).is_file() or not os.access(name, os.X_OK):
            parser.error('executable required: ' + name)
    entries = [e for e in inventory['scenarios'] if args.level in e['levels'] and (not args.select or e['id'] in ids) and (args.host == 'all' or e['host'] == args.host) and (bool(args.select) or (e['recommendation'] == 'run-at-end') == args.run_at_end)]
    if not entries:
        parser.error('selection contains no ' + args.level.upper() + ' scenarios for this host')
    destination = Path(args.output)
    if destination.exists():
        parser.error('output already exists; use a new run directory')
    destination.mkdir(parents=True, mode=0o700)
    startup_blocker = ''
    if Path(args.rust_binary).is_file():
        with tempfile.TemporaryDirectory(prefix='ab-gateway-probe-') as home:
            probe_socket = str(Path(home) / 'ipc.sock')
            with (destination / 'gateway-startup.log').open('wb') as log:
                process = subprocess.Popen([str(Path(args.rust_binary).resolve()), 'start', '--role', 'gateway', '--home', home, '--socket', probe_socket, '--peer-addr', '127.0.0.1:0', '--api-addr', '127.0.0.1:0'], stdout=log, stderr=log, env=dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file'))
                try:
                    deadline = time.monotonic() + 30
                    while process.poll() is None and not Path(probe_socket).exists() and time.monotonic() < deadline:
                        time.sleep(.05)
                    if process.poll() is not None:
                        startup_blocker = f'ab-runtime/ab-applepeer: start --role gateway exited {process.returncode} before ipc/1 readiness'
                    elif not Path(probe_socket).exists():
                        startup_blocker = 'ab-runtime/ab-applepeer: gateway ipc/1 socket unavailable after 30s'
                finally:
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
    no_runner = load_no_runner()
    families = str(Path(args.families_binary).resolve())
    simapple = str(Path(args.simapple_binary).resolve())
    started = time.monotonic()
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel) as pool:
        futures = [pool.submit(execute, e, args.timeout, destination, args.rust_binary, startup_blocker, families, simapple, no_runner) for e in entries]
        for future in concurrent.futures.as_completed(futures):
            row = future.result()
            rows.append(row)
            print(f"{row['id']:<45} {row['status']:<9} {row['seconds']:>9.3f}s", flush=True)
            for blocker in row.get("blocked_on", []):
                print("  blocked-on: " + blocker, flush=True)
            if row['status'] == 'no-runner':
                print("  no-runner: " + row['reason'], flush=True)
            report = {'wall_seconds': round(time.monotonic() - started, 3), 'parallel': args.parallel, 'host': platform.system().lower(), 'complete': len(rows) == len(entries), 'expected': len(entries), 'results': sorted(rows, key=lambda r: r['id'])}
            temporary = destination / 'summary.json.tmp'
            temporary.write_text(json.dumps(report, indent=2) + '\n')
            temporary.replace(destination / 'summary.json')
    print(f"Wall: {report['wall_seconds']:.3f}s; {len(rows)} scenarios", flush=True)
    return int(any(r['status'] not in ('pass', 'no-runner') for r in rows))


if __name__ == '__main__':
    sys.exit(main())
