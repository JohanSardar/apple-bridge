import json
import subprocess
import sys

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--manifest-path", "rust/Cargo.toml", "--format-version", "1", "--locked"
]))
packages = {p["id"]: p["name"] for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
allowed = {
    "wasm-bindgen", "wasm-bindgen-shared", "wasm-bindgen-macro", "wasm-bindgen-macro-support", "bumpalo", "rustversion", "js-sys", "futures-util", "slab", "pin-project-lite", "futures-task", "futures-sink", "futures-macro", "futures-io", "futures-core", "futures-channel", "pem-rfc7468", "zeroize_derive", "defmt", "defmt-macros", "defmt-parser", "pkcs8", "spki", "der", "const-oid", "base64ct", "hex", "hmac", "hkdf", "aes-gcm", "aead", "aes", "cipher", "ctr", "ghash", "polyval", "universal-hash", "opaque-debug", "subtle", "zeroize", "inout", "ed25519-dalek", "ed25519", "signature", "curve25519-dalek", "curve25519-dalek-derive", "x25519-dalek", "rand_core", "getrandom", "rustc_version", "semver", "fiat-crypto", "jiff-core", "wasi", "ab-native", "ab-session", "ab-shortcuts", "ab-domain", "ab-wire", "ab-sys", "serde", "serde_core",
    "serde_derive", "serde_json", "sha2", "thiserror", "thiserror-impl", "jiff", "jiff-static",
    "jiff-tzdb", "jiff-tzdb-platform", "libc", "windows-sys", "windows-link", "windows-targets",
    "windows_aarch64_gnullvm", "windows_aarch64_msvc", "windows_i686_gnu", "windows_i686_gnullvm",
    "windows_i686_msvc", "windows_x86_64_gnu", "windows_x86_64_gnullvm", "windows_x86_64_msvc",
    "proc-macro2", "quote", "syn", "unicode-ident", "itoa", "memchr", "zmij", "ryu",
    "digest", "crypto-common", "block-buffer", "generic-array", "typenum", "version_check",
    "cfg-if", "cpufeatures", "portable-atomic", "portable-atomic-util", "log",
    "sqlite-wasm-rs", "rsqlite-vfs", "rusqlite", "libsqlite3-sys", "cc", "find-msvc-tools", "shlex", "pkg-config", "vcpkg",
    "bitflags", "fallible-iterator", "fallible-streaming-iterator", "hashlink", "hashbrown",
    "foldhash", "smallvec", "flate2", "miniz_oxide", "adler2", "crc32fast", "simd-adler32",
    "plist", "base64", "indexmap", "equivalent", "quick-xml", "time", "time-core", "time-macros",
    "deranged", "num-conv", "powerfmt", "num_threads",
}
failed = False
for root, name in packages.items():
    if name not in {"ab-native", "ab-session"}:
        continue
    pending = [(root, [name])]
    seen = set()
    while pending:
        current, path = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        if packages[current] not in allowed:
            print("offline graph rejects: " + " -> ".join(path), file=sys.stderr)
            failed = True
        for child in nodes[current]["deps"]:
            if all(kind["kind"] == "dev" for kind in child["dep_kinds"]):
                continue
            pending.append((child["pkg"], path + [packages[child["pkg"]]]))
if failed:
    sys.exit(1)
print("native offline dependency graph: OK")
