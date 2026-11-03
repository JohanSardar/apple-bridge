# Gateway persistence parity

Run on Ubuntu with Go and Rust in PATH:

```sh
go build -o /tmp/ab-ga-go ./rust/crates/ab-state/gateway-parity
cargo build --manifest-path rust/Cargo.toml -p ab-state --examples --features lab
python3 rust/crates/ab-state/gateway-parity/run.py
```

`AB_GATEWAY_GO` and `AB_GATEWAY_RUST` select alternate binaries. Set TMPDIR and GOTMPDIR to a private build directory if the shared /tmp quota is exhausted.

The driver sends the golden ipc Capture plus synthetic captures and command sequences to separate Go and Rust processes on cloned synthetic homes. It compares records, aliases, checkpoints, journal payloads, commit boundaries, commands and dedupe rows after mutations. Initial aliases and epoch/era are shared so capture payload bytes can be compared exactly. A separate fresh Rust home exercises random ID creation and Go reopen. The sequence includes no-op observation watermarks, duplicates, reset, reference rollback, cross-generation scoped aliases, stale backfill, incomplete/complete absence, eviction, Notes content versions, snapshot lease retention, approval transitions, CAS rejection, ID reuse, command parts and runner completion. Restore is exercised in both directions and command rows must remain byte-identical to their own pre-restore contents. Loss of a ledger with command evidence refuses startup.

Journal committed_at clocks are excluded. Command-generated event clocks and their mirror observation clocks are replaced by CLOCK for cross-process byte comparison; every other payload byte is compared unchanged. This does not claim literal equality of independently generated wall-clock values, random IDs or SQLite file pages. The raw ledger is never rewritten by the comparator. The fresh-home probe checks interoperability separately from deterministic clone equality.

The lab feature exposes only an SQLite max-page limit for the isolated example driver. It forces real SQLITE_FULL during capture, requires unchanged cursor, checks sources.status capacity degradation, removes the limit and verifies capture recovery. It is absent from default builds.

Evidence also includes the existing 604-comparison replica regression suite. Ubuntu make rust-ci (including make lint, all Rust tests, clippy, cargo-deny and Go golden suites) and clippy with all ab-state features passed. The integrated Rust gateway SIM gate awaits gateway runtime wiring: the current ab-runtime accepts only the replica role. Real smoke and live shadow replay belong to the orchestrator and were not run here.
