import json
import os
import pathlib
import shlex
import sqlite3
import shutil
import subprocess
import sys
import tempfile

local_mode = "--local" in sys.argv
arguments = [argument for argument in sys.argv[1:] if argument != "--local"]
oracle, rust_binary = arguments
data_bytes = os.environ.get("AB_PARITY_DATA_BYTES")
if local_mode and not data_bytes:
    raise SystemExit("--local requires AB_PARITY_DATA_BYTES")
script = pathlib.Path(__file__).with_name("generate.py")
local = pathlib.Path.home() / "bin/ab-local"
remote = None if local_mode else subprocess.check_output(["ssh", "build", "mktemp -d ~/w/phase6-usage-XXXXXX"], text=True).strip()
try:
    with tempfile.TemporaryDirectory(prefix="ab-native-usage-") as temporary:
        root = pathlib.Path(temporary) / "fixture"
        subprocess.run([sys.executable, str(script), str(root)], check=True)
        context_path = root / "context.json"
        context = json.loads(context_path.read_text())
        processes = []
        def sync():
            if not local_mode:
                subprocess.run(["rsync", "-acz", "--delete", str(root) + "/", "build:" + remote + "/"], check=True)
        sync()
        swift_command = [oracle] if local_mode else [str(local), oracle]
        rust_command = [rust_binary, "usage", str(root)] if local_mode else ["ssh", "build", shlex.join([rust_binary, "usage", remote])]
        swift_env = dict(os.environ, TZ=context["zone"])
        processes.append(subprocess.Popen(swift_command + [str(root), "--interactive"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=swift_env))
        processes.append(subprocess.Popen(([rust_binary, "usage", str(root), "--interactive", "--frames"] if local_mode else ["ssh", "build", shlex.join([rust_binary, "usage", remote, "--interactive", "--frames"])]), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True))
        try:
            captures = []
            for stage in range(5):
                context["checkpoint"] = f"synthetic-checkpoint-{stage}"
                context["scan_id"] = f"synthetic-scan-{stage}"
                context_path.write_text(json.dumps(context))
                if stage == 2:
                    path = root / "streams/restricted/App.InFocus/local/1"
                    path.write_bytes(path.read_bytes().replace(b"test.synthetic.beta", b"test.synthetic.zeta"))
                if stage == 3:
                    (root / "streams/restricted/App.InFocus/remote/synthetic-remote/1").unlink()
                if stage == 4:
                    (root / "streams/restricted/App.InFocus/local/1").write_bytes(b"malformed")
                sync()
                frames = []
                raw_frames = []
                for process in processes:
                    process.stdin.write("scan\n")
                    process.stdin.flush()
                    line = process.stdout.readline()
                    if not line:
                        raise RuntimeError("fixture reader exited before capture")
                    raw_frames.append(line)
                    frames.append(json.loads(line))
                if frames[0] != frames[1]:
                    raise AssertionError(f"stage {stage}: Swift {frames[0]} != Rust {frames[1]}")
                digests = subprocess.check_output(([data_bytes] if local_mode else ["ssh", "build", "~/targets/phase6-native/fixture-data-bytes"]), input="".join(raw_frames), text=True).splitlines()
                assert len(digests) == 2 and digests[0] == digests[1], f"data byte mismatch at stage {stage}"
                captures.append(frames[0])
            assert captures[1]["observations"] == []
            assert captures[-1]["completion_evidence"] == "complete"
            print(f"Screen Time synthetic streams: {len(captures)} checkpoints, zero differences in observations and Go JSON data hashes")
        finally:
            for process in processes:
                process.stdin.close()
            for process in processes:
                if process.wait(timeout=15) != 0:
                    raise RuntimeError("fixture reader failed")
        for failure in ["missing-local", "missing-schema"]:
            if failure == "missing-local":
                shutil.rmtree(root / "streams/restricted/App.InFocus/local")
            else:
                (root / "streams/restricted/App.InFocus/local").mkdir()
                with sqlite3.connect(root / "sync/sync.db") as database:
                    database.execute("DROP TABLE DevicePeer")
            sync()
            checks = [
                (swift_command + [str(root)], swift_env),
                (rust_command, None),
            ]
            for command, environment in checks:
                result = subprocess.run(command, env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
                assert result.returncode != 0 and not result.stdout, f"{failure}: incomplete input emitted a capture"
        print("Screen Time failed-input controls: both readers emitted zero captures")
finally:
    if not local_mode:
        subprocess.run(["ssh", "build", "rm -rf -- " + shlex.quote(remote)], check=True)
