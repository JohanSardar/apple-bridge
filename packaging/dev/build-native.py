#!/usr/bin/env python3
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

package = Path(sys.argv[1] if len(sys.argv) > 1 else "native").resolve()
output = package / ".build" / "clt-release"
output.mkdir(parents=True, exist_ok=True)
sdk = os.environ.get("APPLE_BRIDGE_SDK", "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk")
compiler = os.environ.get("APPLE_BRIDGE_SWIFTC", "swiftc")
manifest_api = Path(subprocess.check_output(["xcrun", "--find", "swiftc"], text=True).strip()).resolve().parent.parent / "lib/swift/pm/ManifestAPI"
manifest = output / "package-manifest"
subprocess.run([compiler, "-sdk", sdk, str(package / "Package.swift"), "-I", str(manifest_api), "-L", str(manifest_api), "-lPackageDescription", "-package-description-version", "6.3.0", "-o", str(manifest)], check=True)
environment = dict(os.environ, DYLD_LIBRARY_PATH=str(manifest_api))
metadata = json.loads(subprocess.check_output([str(manifest), "-fileno", "1"], env=environment, text=True))
if metadata["errors"] or metadata["package"]["dependencies"]:
    raise SystemExit("Only local package targets are supported by the CLT fallback")
targets = {t["name"]: t for t in metadata["package"]["targets"]}
built = set()
visiting = set()
objects = {}
clang_modules = []

def build(name):
    if name in built:
        return
    if name in visiting:
        raise SystemExit("Cyclic target dependency: " + name)
    visiting.add(name)
    target = targets[name]
    dependencies = []
    for dependency in target["dependencies"]:
        value = dependency.get("byName", dependency.get("target", {})).get("name")
        if value not in targets:
            raise SystemExit("Unsupported target dependency")
        dependencies.append(value)
        build(value)
    sources = package / target.get("path", "Sources/" + name)
    excluded = set(target.get("exclude", []))
    files = sorted(str(p) for p in sources.rglob("*.swift") if str(p.relative_to(sources)) not in excluded)
    objc = sorted(p for p in sources.rglob("*.m") if str(p.relative_to(sources)) not in excluded)
    if objc:
        if files or "regular" not in target["type"]:
            raise SystemExit("Objective-C targets must be separate library targets")
        headers = sources / target.get("publicHeadersPath", "include")
        module_map = output / (name + ".modulemap")
        declarations = "\n".join('header "' + str(p) + '"' for p in sorted(headers.rglob("*.h")))
        module_map.write_text("module " + name + " {\n" + declarations + "\nexport *\n}\n")
        clang_modules.append(str(module_map))
        closure = set()
        for dependency in dependencies:
            closure.update(objects[dependency])
        for source in objc:
            obj = output / (name + "-" + source.stem + ".o")
            subprocess.run(["xcrun", "clang", "-isysroot", sdk, "-target", platform.machine() + "-apple-macosx26.0", "-fobjc-arc", "-fmodules", "-O", "-Wall", "-Werror", "-I", str(headers), "-c", str(source), "-o", str(obj)], check=True)
            closure.add(str(obj))
        objects[name] = closure
        visiting.remove(name)
        built.add(name)
        return
    flags = [compiler, "-sdk", sdk, "-target", platform.machine() + "-apple-macosx26.0", "-swift-version", "6", "-warnings-as-errors", "-O", "-parse-as-library", "-I", str(output), "-module-name", name]
    for module_map in clang_modules:
        flags += ["-Xcc", "-fmodule-map-file=" + module_map]
    closure = set()
    def collect(item):
        closure.update(objects[item])
    for dependency in dependencies:
        collect(dependency)
    if "regular" in target["type"]:
        obj = output / (name + ".o")
        subprocess.run(flags + ["-emit-module", "-emit-object", "-whole-module-optimization", "-emit-module-path", str(output / (name + ".swiftmodule"))] + files + ["-o", str(obj)], check=True)
        closure.add(str(obj))
    elif "executable" in target["type"]:
        subprocess.run(flags + files + sorted(closure) + ["-o", str(output / name)], check=True)
    else:
        raise SystemExit("Unsupported target type: " + name)
    objects[name] = closure
    visiting.remove(name)
    built.add(name)

for name in targets:
    build(name)
