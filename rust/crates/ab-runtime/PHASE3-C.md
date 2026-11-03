# Phase 3 C integration

The runtime uses the merged State, Trust, Replication and Blobs crates. `replica-feed` is enabled by default. The home singleton lock precedes State/feed startup; actual API and inbound feed listen addresses are persisted. Commands use B's authenticated transport for direct mTLS and gateway-dials sessions, with cancellation and same-ID lookup recovery. Direct requests reuse a client keyed by the pinned gateway fingerprint, with deadlines applied per request. Connected readiness waits for the durable control acknowledgement to be flushed and, when ntp/1 is negotiated, for a matching clock pong sent after that acknowledgement. This proves the gateway has consumed the control receipt before commands or connected status are exposed.

REST and MCP use the same operations service. Specialized reads include contacts lookup, Health, paired iPhones and both status projections. Legacy result serialization follows Go struct field order, including nested read models. Command envelopes, descriptor argument shapes, required versions and wait bounds are checked before forwarding. The Go gateway remains the command/expiry authority; caller expiry and admission-era values are forwarded unchanged so retries retain lookup semantics.

Doctor opens SQLite read-only and checks integrity, journal application, shape violations, recursive private modes, versions, storage encryption and authenticated four-timestamp clock samples. Status is read-only. CLI retention durations use Go units; SIGINT/SIGTERM initiate graceful shutdown with a ten-second drain bound.

Query blob grants use the frozen SQLite tables, are principal/scope/expiry checked, and authorize verified cached blob reads. Uncached query blobs are fetched in bounded signed chunks, digest verified and persisted; range responses are supported.

## Validation on Ubuntu

The exit gate passed at implementation commit `4ce8f25efc7ce2002cc7d8a770e5aa43ed375090`, rebased onto main `38368b817a0974a4377c668a6faa13449bbc0bee`. The frozen lab binary SHA-256 is `6efd507baab2c2fceae6959d9355bd253219e918b9b7cc4089faa88b52f2d17b`.

- `make replica-matrix`, with `gateway=go,replica=rust`, the frozen binary and `--local-replicas --jobs 1`: **25 passed, 0 failed**, exit 0. The Ubuntu selection consists of byte-identical copies of the 25 runnable scenario YAMLs; their original paths and hashes are recorded. This PR changes no existing scenarios or frozen corpora.
- Mixed convergence passed: the Go gateway, Go replica and Rust replica each held 103 records with digest `da6b5754814c01477abab631e8ffe2e40a1f8379663094b15b5041df7a498d0a`.
- `go run ./lab/replica-parity <frozen Rust binary>`: **21 REST + 21 MCP comparisons passed**. Deterministic bytes match, including raw read-model field order and MCP structuredContent/text bytes. Live heartbeat, staleness, local replica observation timestamps and MCP read-model duration are checked separately as authorized. The probe covers nonempty blob status, actual command forwarding with independently inspected Go gateway ledger disposition (`locally_observed` on both sides), active REST/MCP cancellation, three range forms and digest-verified offline query blob reads.
- The real product-adapter frontdoor passes **30 checks**, including all four unchanged golden MCP exchanges through JSON and SSE, public authorization/limits, legacy initialization and cancellation notifications. Its unauthenticated golden route is a lab-only internal-handler seam matching the Go golden fixture. Production routes remain authenticated.
- `make rust-ci`, lab-feature workspace Clippy and `go vet ./lab/replica-parity` passed. The full matrix's maximum echo-query latency was 59.18 ms against the unchanged 200 ms budget.
- The harness retains ten optional `replica_application_exactly_once` assertions as `unverified` because those journeys do not run kill-during-apply. These are preserved in the receipts, not counted as passed assertions. MAIL-REPLICA leaves its per-scenario git SHA empty; its run is bound to the same frozen artifact by the matrix invocation, and that metadata gap is preserved.
- The requested **13 native scenarios are pending Air real run**, unrun. Nine additional remote/relay harness blockers are also unrun. Their exact IDs are listed in the receipt.

[Exit-gate receipt](evidence/phase3/exit-gate.json), [matrix log](evidence/phase3/matrix.log), [read-parity log](evidence/phase3/read-parity.log), [mixed convergence](evidence/phase3/mixed-convergence.json), [scenario hashes](evidence/phase3/scenario-selection.json), [MCP receipt](evidence/phase3/mcp.json).

## Commands

Production binary (on Ubuntu):

```
CARGO_TARGET_DIR=$HOME/ab-cargo-target cargo build --manifest-path rust/Cargo.toml --bin bridge
```

Harness binary adds `--features ab-runtime/lab-queries`. `make replica-matrix` selects that feature automatically. Set `APPLE_BRIDGE_LAB_ROOT` and `TMPDIR` to directories on the Ubuntu home filesystem when `/tmp` quotas prevent staging executables. This override preserves the default harness root and all assertions. Go 1.27.3 was installed from the official archive under `~/go-sdk`; put `~/go-sdk/bin` and `~/.cargo/bin` on PATH.
