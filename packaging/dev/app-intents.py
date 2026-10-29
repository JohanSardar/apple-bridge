#!/usr/bin/env python3
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[2]
source = root / "native/Sources/AppleBridgeHelper/BridgeStatusIntent.swift"
resources = root / "packaging/app-intents"
digest = hashlib.sha256(source.read_bytes()).hexdigest()
if sys.argv[1:] == ["--generate"]:
    with tempfile.TemporaryDirectory(prefix="ab-app-intents-") as temporary:
        work = Path(temporary)
        sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
        compiler = Path(subprocess.check_output(["xcrun", "--find", "swiftc"], text=True).strip())
        version = subprocess.check_output(["xcodebuild", "-version"], text=True).split()[-1]
        triple = platform.machine() + "-apple-macosx26.0"
        protocols = work / "protocols.json"
        protocols.write_text(json.dumps(["AppIntent", "AppShortcutsProvider"]))
        constants = work / "intent.swiftconstvalues"
        subprocess.run([str(compiler), "-sdk", sdk, "-target", triple, "-parse-as-library", "-module-name", "AppleBridgeHelper", "-emit-object", "-o", str(work / "intent.o"), "-emit-const-values-path", str(constants), "-Xfrontend", "-const-gather-protocols-file", "-Xfrontend", str(protocols), str(source)], check=True)
        (work / "sources").write_text(str(source) + "\n")
        (work / "constants").write_text(str(constants) + "\n")
        subprocess.run(["xcrun", "appintentsmetadataprocessor", "--output", str(work), "--toolchain-dir", str(compiler.parent.parent), "--module-name", "AppleBridgeHelper", "--sdk-root", sdk, "--xcode-version", version, "--platform-family", "macOS", "--deployment-target", "26.0", "--target-triple", triple, "--source-file-list", str(work / "sources"), "--swift-const-vals-list", str(work / "constants")], check=True)
        resources.mkdir(parents=True, exist_ok=True)
        shutil.copytree(work / "Metadata.appintents", resources / "Metadata.appintents", dirs_exist_ok=True)
        (resources / "source.sha256").write_text(digest + "\n")
else:
    if (resources / "source.sha256").read_text().strip() != digest:
        raise SystemExit("App Intents changed; run packaging/dev/app-intents.py --generate with Xcode before building")
    destination = Path(sys.argv[1]) / "Contents/Resources/Metadata.appintents"
    shutil.copytree(resources / "Metadata.appintents", destination)
