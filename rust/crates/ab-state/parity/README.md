> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# Replica compatibility evidence

This is a synthetic, process-driven compatibility scenario for worker A. It drives Go's real State/Operations APIs and Rust's `State<Replica>` through separate line-oriented binaries; it is not the Phase 3 mixed gateway/replica scenario gate owned by worker D.

On Ubuntu from the repository root:

```sh
cargo build --release --manifest-path rust/Cargo.toml -p ab-state --example state_probe
go build -o /tmp/ab-state-go ./rust/crates/ab-state/parity
python3 rust/crates/ab-state/parity/run.py
```

The driver creates two private temporary homes, installs identical synthetic corpora, compares complete JSON bytes, swaps the binaries between homes, and repeats. Rust output uses `ab-wire::gojson`; Go output is re-marshaled through its map encoder to compare object keys in the same order. This compares result content and number/string encoding while cursor-token bytes remain untouched. It does not certify the declaration order of dynamic JSON read-model results.

The corpus contains 412 records: messages, reactions, attachment/blob references, contacts, reminders, notes, calendar events, a source-status record, and a Health sample. The scenario covers paging, FTS, all chat filters, exact participants, hidden contact links, reminder smart views and sort modes, specialized reads, both reopen directions, scope restriction, digest/count failures, epoch/revision mismatches, gaps/overlaps, partial capture visibility, a partial capture abandoned on restart, and absent-record removal. Both homes are checked for schema v2, integrity, and mode 0600. Temporary data is deleted automatically.

`evidence.json` records 604 byte-equal comparisons with zero mismatches and `impl=rust`. Endpoint timings are local pipe request/response times including query execution, JSON conversion and serialization, for optimized Go and release Rust binaries on Ubuntu. Each operation mixes the query variants above. The small corpus fits in memory; measurements do not certify the harness's 100k-message or 1000-reminder budgets, network latency, or production datasets. Sample counts and p50/p95/max are recorded per operation.

Checks run on Ubuntu: workspace Rust tests, strict workspace Clippy, cargo-deny, repository lint and Go golden suites pass. `cargo fmt -p ab-state -- --check` passes. `make rust-ci` stops at an existing formatting difference in `ab-wire/src/lib.rs` (module order); this branch leaves worker-owned shared-core code untouched.

Reminder title sorting uses pinned ICU collation with secondary strength and numeric ordering, following the [ICU CollatorPreferences contract](https://docs.rs/icu_collator/2.1.1/icu_collator/struct.CollatorPreferences.html). Corpus parity includes numbered titles; exhaustive cross-version Unicode collation parity remains unverified.

Runtime must supply current replica progress through `set_replica_status`; State scopes its blob entries to visible records. Worker C should dispatch contact lookup and status operations through `read_model`, as detailed in `rust/INTERFACES.md`.
