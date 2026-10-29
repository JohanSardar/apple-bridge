#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

package = Path(sys.argv[1] if len(sys.argv) > 1 else "native").resolve()
lab = (os.environ.get("APPLE_BRIDGE_NATIVE_LAB") or "1")
if lab not in {"0", "1"}:
    raise SystemExit("APPLE_BRIDGE_NATIVE_LAB must be 0 or 1")
output = package / ".build" / ("clt-release" if lab == "1" else "clt-product-release")
output.mkdir(parents=True, exist_ok=True)
sdk = os.environ.get("APPLE_BRIDGE_SDK", "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk")
compiler = os.environ.get("APPLE_BRIDGE_SWIFTC", "swiftc")
manifest_api = Path(subprocess.check_output(["xcrun", "--find", "swiftc"], text=True).strip()).resolve().parent.parent / "lib/swift/pm/ManifestAPI"
manifest = output / "package-manifest"
stamps = {}


def relocatable(text):
    return str(text).replace(str(package), "@package")

def digest(parts):
    value = hashlib.sha256()
    for part in parts:
        value.update(part if isinstance(part, bytes) else str(part).encode())
        value.update(b"\0")
    return value.hexdigest()


def current(name, key, outputs):
    stamp = output / (name + ".stamp")
    return stamp.exists() and stamp.read_text() == key and all(Path(o).exists() for o in outputs)


def record(name, key):
    (output / (name + ".stamp")).write_text(key)


manifest_key = digest([compiler, sdk, manifest_api, (package / "Package.swift").read_bytes()])
if not current("package-manifest", manifest_key, [manifest]):
    subprocess.run([compiler, "-sdk", sdk, str(package / "Package.swift"), "-I", str(manifest_api), "-L", str(manifest_api), "-lPackageDescription", "-package-description-version", "6.3.0", "-o", str(manifest)], check=True)
    record("package-manifest", manifest_key)
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
            key = digest([sdk, source.read_bytes()] + [h.read_bytes() for h in sorted(headers.rglob("*.h"))])
            stamps[name] = digest([stamps.get(name, ""), key])
            closure.add(str(obj))
            if current(name + "-" + source.stem, key, [obj]):
                continue
            record(name + "-" + source.stem, "")
            subprocess.run(["xcrun", "clang", "-isysroot", sdk, "-target", platform.machine() + "-apple-macosx26.0", "-fobjc-arc", "-fmodules", "-O", "-Wall", "-Werror", "-I", str(headers), "-c", str(source), "-o", str(obj)], check=True)
            record(name + "-" + source.stem, key)
        objects[name] = closure
        visiting.remove(name)
        built.add(name)
        return
    flags = [compiler, "-sdk", sdk, "-target", platform.machine() + "-apple-macosx26.0", "-swift-version", "6", "-warnings-as-errors", "-O", "-parse-as-library", "-I", str(output), "-module-name", name]
    if lab == "1":
        flags += ["-D", "LAB"]
    for module_map in clang_modules:
        flags += ["-Xcc", "-fmodule-map-file=" + module_map]
    closure = set()
    def collect(item):
        closure.update(objects[item])
    for dependency in dependencies:
        collect(dependency)
    key = digest([relocatable(x) for x in flags] + [stamps[d] for d in dependencies] + [Path(f).read_bytes() for f in files] + [relocatable(f) for f in files])
    stamps[name] = key
    if "regular" in target["type"]:
        obj = output / (name + ".o")
        closure.add(str(obj))
        if not current(name, key, [obj, output / (name + ".swiftmodule")]):
            record(name, "")
            subprocess.run(flags + ["-emit-module", "-emit-object", "-whole-module-optimization", "-emit-module-path", str(output / (name + ".swiftmodule"))] + files + ["-o", str(obj)], check=True)
            record(name, key)
    elif "executable" in target["type"]:
        if not current(name, key, [output / name]):
            record(name, "")
            subprocess.run(flags + files + sorted(closure) + ["-o", str(output / name)], check=True)
            record(name, key)
    else:
        raise SystemExit("Unsupported target type: " + name)
    objects[name] = closure
    visiting.remove(name)
    built.add(name)

for name in ([os.environ["APPLE_BRIDGE_NATIVE_TARGET"]] if os.environ.get("APPLE_BRIDGE_NATIVE_TARGET") else targets):
    build(name)
