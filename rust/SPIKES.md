# Phase 1 Rust spikes

Measured 2026-11-02; initially against Go main `10a397d6`, then updated and reverified against Phase 0 main `a02485dd`. No gateway deployment or live-home writes. Database comparisons used online `.backup` snapshots, read-only connections on Ubuntu, and SHA-256 comparisons only. Both local and remote database clones were deleted after comparison. No record or command payloads are retained here.

## S1: Air build budget

**PASS** for the measured skeleton at jobs 1, 2 and 3. All sampled peaks are below 3.5 GB, all leaf checks are below 30 s, and all targets are below 6 GB.

| Jobs | Cold command | Wall seconds | Peak tree RSS MiB | Target MiB | Leaf check seconds |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | check | 142.563 | 636.2 | 207.2 | 13.088 |
| 1 | build | 113.846 | 701.3 | 434.9 | 15.684 |
| 2 | check | 64.790 | 794.0 | 207.2 | 8.198 |
| 2 | build | 73.323 | 831.4 | 434.9 | 11.501 |
| 3 | check | 48.203 | 942.5 | 207.2 | 8.714 |
| 3 | build | 60.334 | 1094.2 | 434.9 | 8.883 |

Recommend **jobs=2**, one shared target directory, line-table workspace debug information and no dependency debug information. At jobs=2: cold check 64.790 s, cold build 73.323 s, peak build RSS 0.812 GiB, leaf check at most 11.501 s, target 0.425 GiB. Keep Linux builds on Ubuntu and serialize each Air Cargo invocation with `ab-local`, releasing the lock between steps. Jobs=3 passes this skeleton but uses more memory; it does not establish a budget for the future full product. Exact observations are in `spikes/build-budget.json`.


Method: macOS 26.7.1, 8 GiB RAM, 8 logical CPUs, Rust/Cargo 1.99.1. Native arm64 development builds, dependency debug info disabled and workspace line tables enabled. All builds use `ab-local`, `CARGO_BUILD_JOBS=2`, and the shared `~/ab-cargo-target`; explicit `-j 1/2/3` selects the experimental concurrency. `spike-budget` inherits `dev` and isolates cleanup from other workers' profiles. The shared Cargo registry cache is reused, but the measured profile is cleaned before each cold check/build. Registry downloads are not timed separately and may contribute to the first cold observation. The realistic dependency set includes every dependency requested, plus the certificate parser used by S7. These are end-to-end Cargo command timings, not CPU-only compiler timings.

Peak memory is sampled aggregate RSS of Cargo and its descendant processes at 100 ms intervals, excluding unrelated workers. This is a sampled peak, not an OS high-water mark; shared pages may be counted more than once and brief peaks can be missed. Wall time excludes waiting for the build lock and includes sampling overhead. Target disk use is allocated bytes from `du -sk`, for this profile only. A leaf check touches `ab-wire/src/lib.rs` then checks `ab-wire`. Each row has n=1; its p50/p95/max equal the reported observation, with no statistical confidence implied. The initial two rows and the jobs=2 check ran within one lock acquisition; subsequent steps acquire the lock separately so other workers can interleave.

Reproduce from the workspace: `python3 rust/spikes/budget.py`. The controller must run outside `ab-local`; it acquires the lock separately for every clean/check/build/leaf check. Results resume from `/tmp/ab-rust-budget.json`; remove that results file to start a new matrix. The final cleanup removes only `spike-budget` artifacts.

## S3: gojson

**PASS for the requested synthetic and live-database comparisons. The full plan exit gate remains INCOMPLETE until recorded capture frames are supplied.**

- Generated vectors: **10,042**, **0 mismatches**. Includes 10,000 deterministic exponent/fraction cases, huge and tiny exponents, integers above 2^53, negative zero, duplicate keys, HTML modes, control characters, Unicode separators, lone surrogate keys/values, surrogate pairs, escaped backslashes, and three invalid UTF-8 byte sequences. The invalid UTF-8 cases transport original bytes separately so JSON transport cannot mask replacement differences. Typed command vectors cover year 0001, timezone-to-UTC conversion, nanoseconds/trailing-zero trimming, and maximum u64 versions.
- Live `records.Data`: **195,477 records**, **0 mismatches** against Go `UseNumber` decode and re-marshal.
- Live ledger commands: **11 rows**, **0 mismatches** against the actual `contracts.CommandHash` implementation. Comparison is of recomputed Go and Rust hashes, not a claim about historical stored request hashes.
- `ab-wire::gojson` preserves number tokens using `RawValue`, orders maps, applies Go HTML/no-HTML escaping, replaces unpaired UTF-16 surrogates, formats UTC `GoTime`, and encodes the capitalized command fields in declaration order. It deliberately does not expose a universal struct canonicalizer.
- `tools/lint` rejects the two prohibited serde_json features in manifests. `cargo deny` also rejects their resolved activation, including through transitive dependencies.

Reproduce synthetic checks: build `rust/spikes` and `go build -o <oracle> ./tools/rust-spike`, then `<oracle> <ab-spikes>`. Optional following database paths invoke the hash-only comparison. Clone with `sqlite3 -readonly`: begin a read transaction, read `sqlite_schema` to establish its snapshot, invoke `.backup` into an owner-only temporary directory, then roll back. The explicit read transaction prevents online-backup restarts under active capture. Never pass a live database to the comparator. No raw capture recording was enabled or read from the gateway.

**Phase 0 golden verification passed after it landed on main `a02485dd`: 22 compatibility comparisons, 0 mismatches, in two Rust integration tests.** These check all four JSON vectors in both escape modes, all four command preimages and hashes, five timestamp normalizations, and the Capture fixture observation Data. This fixture is synthetic; it does not replace a recorded live-capture corpus. Reproduce with `cargo test --manifest-path rust/Cargo.toml -p ab-wire`.

## S5: SQLite parity

**PASS.** Rust `rusqlite=0.41.1`, bundled `libsqlite3-sys=0.39.0`, runtime SQLite **3.54.0**; Go modernc runtime SQLite **3.53.4**.

- Rust compile options: `ENABLE_FTS5=true`, `OMIT_WAL=false`.
- **8 queries, 0 mismatches**, exact ordered row IDs on a five-document synthetic corpus. Both use `unicode61 remove_diacritics 2`; queries cover accents, case, phrases, prefix, Japanese, Boolean OR, and no hits.
- WAL snapshot isolation: a pinned reader retained its count across a committed write from a second connection, then saw the additional row after committing its read transaction. **0 isolation mismatches**.
- Go `state.Open` created the actual schema-v2 state database; Rust reopened it, queried FTS and wrote another document; Go reopened it, observed **6 documents**, and verified `schema_version=2`. **0 reopen mismatches**. This tests state.db, not a migration implementation or live-ledger write compatibility.

Reproduce with `<oracle> sqlite <temporary-path-prefix> <ab-spikes>`. Use a fresh prefix; the probe creates a synthetic home. All synthetic database artifacts from this run are cleaned after verification.

## S7: mTLS

**PASS for the requested mTLS scope.** Rustls **0.23.46**, ring provider, TLS 1.3 only, generated Ed25519 identities, SHA-256 of complete certificate DER, and validity checks. TLS handshake signatures are verified using ring; the custom certificate verifier does not bypass proof of private-key possession.

**10 cases, 0 mismatches**: Go server/Rust client and Rust server/Go client both exchanged application bytes; four wrong-pin cases and four expired-certificate cases were rejected. Go uses the actual `internal/trust` `PinnedListener` and `ClientTLS` implementations. Synthetic identities are generated in a temporary directory and deleted; no live credentials were accessed. The plan's separate smux/relay component is outside this requested mTLS spike and remains unverified.

Reproduce with `python3 rust/spikes/tls_matrix.py <oracle> <ab-spikes>`.

## Gates and cleanup

Ubuntu: `make lint`, `go test ./...`, Rust build, `cargo clippy --all-targets -- -D warnings`, and `cargo deny check` passed. Cargo-deny reports advisories, licenses, bans and sources OK; duplicate-version warnings are retained rather than suppressed. Rust formatting uses `cargo fmt`. The zero-comment lint has checks for raw/ordinary strings, characters, forbidden comments, the restricted SAFETY exception, doc attributes and the feature bans.

All six Air measurements completed, and the `spike-budget` profile was cleaned (2,045 files / 467.3 MiB removed). Existing shared profiles were preserved. Final golden checks and Rust/Go gates include Phase 0 main `a02485dd` and relay containment main `c76a25d6`. Database clones and temporary synthetic identities are deleted; the isolated Ubuntu workspace, build artifacts and Go toolchain are removed after final verification. The final fresh-clone comparison used the final UTF-8 decoder and observed seven more records than the initial comparison, still with zero mismatches. The plan's 25 GB free-space prerequisite was not met; the task explicitly requested measurements on the available Air. No unrelated worktrees or caches were pruned. These results do not certify the whole migration or authorize a gateway cutover.

Crate/API references: [rusqlite](https://docs.rs/rusqlite/0.41.1/rusqlite/), [rustls certificate verifier](https://docs.rs/rustls/0.23.46/rustls/client/danger/trait.ServerCertVerifier.html), [rmcp features](https://docs.rs/rmcp/3.8.0/rmcp/).
