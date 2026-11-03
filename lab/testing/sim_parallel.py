import argparse
import concurrent.futures
import json
import os
import re
import tempfile
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time


def execute(entry, binary, timeout, logs, implementation, rust_binary, startup_blocker):
    started = time.monotonic()
    target = logs / (entry['id'] + '.log')
    if 'gateway=rust' in implementation and re.search(r'^impl: go-only$', Path(entry['path']).read_text(), re.M):
        return {'id': entry['id'], 'seconds': 0, 'status': 'excluded', 'implementation': implementation, 'reason': 'FINAL-PLAN §1 drops embedded tsnet login/Funnel flow'}
    if 'gateway=rust' in implementation:
        blockers = []
        if startup_blocker:
            blockers.append(startup_blocker)
        if not rust_binary or not Path(rust_binary).is_file():
            blockers.append('ab-runtime: Rust bridge executable unavailable')
        if blockers:
            target.write_text('\n'.join(blockers) + '\n')
            return {'id': entry['id'], 'seconds': 0, 'status': 'blocked', 'execution_status': 'not-started', 'implementation': implementation, 'required_features': entry['packages'], 'blocked_on': blockers, 'harness_audit': gateway_blockers(entry)}
    with target.open('wb') as output:
        command = [binary, 'run', entry['path'], '--level', 'sim', '--prebuilt', '--json']
        command += ['--impl', implementation]
        if rust_binary:
            command += ['--rust-binary', rust_binary]
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
    row = {'id': entry['id'], 'seconds': round(time.monotonic() - started, 3), 'status': status, 'implementation': implementation}
    if 'gateway=rust' in implementation:
        blockers = list(scenario_blockers)
        row['harness_audit'] = gateway_blockers(entry)
        if startup_blocker:
            blockers.append(startup_blocker)
        if not rust_binary or not Path(rust_binary).is_file():
            blockers.append('ab-runtime: Rust bridge executable unavailable')
        row['required_features'] = entry['packages']
        row['blocked_on'] = blockers
        if blockers:
            row['execution_status'] = status
            row['status'] = 'blocked'
    return row


def gateway_blockers(entry):
    blockers = ['harness: semantic product helper ' + call for call in entry.get('semantic_contract_calls', [])]
    for name in entry.get('code_paths', []):
        path = Path(name)
        if not path.is_file():
            continue
        source = path.read_text()
        dependencies = sorted(set(re.findall(r'"github.com/JohanSardar/apple-bridge/internal/(?!contracts")([^"/]+)"', source)))
        if dependencies:
            blockers.append('harness: ' + name + ' still imports product implementations: ' + ', '.join(dependencies))
    return blockers


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--parallel', type=int, default=1 if platform.system() == 'Darwin' else 4)
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--select', action='append', default=[])
    parser.add_argument('--run-at-end', action='store_true')
    parser.add_argument('--host', choices=['linux', 'darwin', 'all'], default=platform.system().lower())
    parser.add_argument('--binary', default='bin/bridge-lab')
    parser.add_argument('--impl', default='gateway=go,replica=go')
    parser.add_argument('--rust-binary', default='')
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
    startup_blocker = ''
    if 'gateway=rust' in args.impl and args.rust_binary and Path(args.rust_binary).is_file():
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
    binary = str(binary_path)
    started = time.monotonic()
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel) as pool:
        futures = [pool.submit(execute, e, binary, args.timeout, destination, args.impl, args.rust_binary, startup_blocker) for e in entries]
        for future in concurrent.futures.as_completed(futures):
            row = future.result()
            rows.append(row)
            print(f"{row['id']:<45} {row['status']:<9} {row['seconds']:>9.3f}s", flush=True)
            for blocker in row.get("blocked_on", []):
                print("  blocked-on: " + blocker, flush=True)
            report = {'wall_seconds': round(time.monotonic() - started, 3), 'parallel': args.parallel, 'host': platform.system().lower(), 'complete': len(rows) == len(entries), 'expected': len(entries), 'results': sorted(rows, key=lambda r: r['id'])}
            temporary = destination / 'summary.json.tmp'
            temporary.write_text(json.dumps(report, indent=2) + '\n')
            temporary.replace(destination / 'summary.json')
    print(f"Wall: {report['wall_seconds']:.3f}s; {len(rows)} scenarios", flush=True)
    return int(any(r['status'] not in ('pass', 'excluded') for r in rows))


if __name__ == '__main__':
    sys.exit(main())
