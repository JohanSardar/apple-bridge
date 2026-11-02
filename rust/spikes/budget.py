import json
import os
from pathlib import Path
import subprocess
import time

root = Path(__file__).resolve().parents[1]
env = dict(os.environ, CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR=str(Path.home() / 'ab-cargo-target'))
target = Path(env['CARGO_TARGET_DIR']) / 'spike-budget'
results = []

def run(args):
    start = time.monotonic()
    with open('/tmp/ab-rust-budget-build.log', 'w') as log:
        process = subprocess.Popen(args, cwd=root, env=env, stdout=log, stderr=log)
        peak = 0
        while process.poll() is None:
            rows = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss='], text=True)
            table = [list(map(int, line.split())) for line in rows.splitlines()]
            descendants = {process.pid}
            for _ in range(12):
                descendants.update(pid for pid, parent, _ in table if parent in descendants)
            peak = max(peak, sum(rss * 1024 for pid, _, rss in table if pid in descendants))
            time.sleep(0.1)
        if process.returncode:
            raise RuntimeError('build failed; see /tmp/ab-rust-budget-build.log')
    return dict(wall_s=round(time.monotonic()-start, 3), peak_tree_rss_bytes=peak)

if __name__ == '__main__':
    import sys
    wrapper = str(Path.home() / 'bin/ab-local')
    script = str(Path(__file__).resolve())
    if len(sys.argv) > 1:
        print(json.dumps(run(sys.argv[1:])))
        raise SystemExit(0)
    results_path = Path('/tmp/ab-rust-budget.json')
    results = json.loads(results_path.read_text()) if results_path.exists() else []
    for jobs in [1, 2, 3]:
        for command in ['check', 'build']:
            if any(row['jobs'] == jobs and row['command'] == command for row in results):
                continue
            subprocess.run([wrapper, 'cargo', 'clean', '--profile', 'spike-budget'], cwd=root, env=env, check=True, capture_output=True)
            measured = json.loads(subprocess.check_output([wrapper, sys.executable, script, 'cargo', command, '--profile', 'spike-budget', '-j', str(jobs)], cwd=root, env=env, text=True))
            measured.update(jobs=jobs, command=command, target_bytes=int(subprocess.check_output(['du', '-sk', str(target)]).split()[0])*1024)
            (root / 'crates/ab-wire/src/lib.rs').touch()
            measured['leaf'] = json.loads(subprocess.check_output([wrapper, sys.executable, script, 'cargo', 'check', '-p', 'ab-wire', '--profile', 'spike-budget', '-j', str(jobs)], cwd=root, env=env, text=True))
            results.append(measured)
            results_path.write_text(json.dumps(results, indent=2))
    subprocess.run([wrapper, 'cargo', 'clean', '--profile', 'spike-budget'], cwd=root, env=env, check=True)
