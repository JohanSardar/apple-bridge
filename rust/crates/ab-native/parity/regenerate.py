import json
import os
import pathlib
import subprocess
import tempfile

root = pathlib.Path(__file__).resolve().parents[4]
local = pathlib.Path.home() / "bin/ab-local"
module = root / "native/.build/decoder-checks"
subprocess.run([str(local), "packaging/dev/check-decoders.sh"], cwd=root, check=True, env=dict(os.environ, APPLE_BRIDGE_DECODER_OUTPUT=str(module)))
with tempfile.TemporaryDirectory(prefix="ab-native-oracle-") as temporary:
    binary = pathlib.Path(temporary) / "decoder-oracle"
    subprocess.run([
        str(local), "swiftc", "-sdk", "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk",
        "-target", "arm64-apple-macosx26.0", "-I", str(module),
        str(pathlib.Path(__file__).with_name("decoders.swift")),
        str(module / "Decoders.o"), "-o", str(binary),
    ], cwd=root, check=True)
    output = subprocess.check_output([
        str(local), str(binary), "lab/fixtures/messages", "lab/fixtures/notes", "lab/fixtures/usage",
    ], cwd=root, text=True)
    groups = {"typedstream": [], "decoders": []}
    for line in output.splitlines():
        vector = json.loads(line)
        group = "decoders" if "decoder" in vector else "typedstream"
        groups[group].append(line)
    for name, lines in groups.items():
        pathlib.Path(__file__).with_name(name + ".jsonl").write_text("\n".join(lines) + "\n")
