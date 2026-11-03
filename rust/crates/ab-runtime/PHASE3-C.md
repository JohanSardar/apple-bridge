# Phase 3 C integration

The runtime uses the merged State, Trust, Replication and Blobs crates. `replica-feed` is enabled by default. The home singleton lock precedes State/feed startup; actual API and inbound feed listen addresses are persisted. Commands use B's authenticated transport for direct mTLS and gateway-dials sessions, with cancellation and same-ID lookup recovery.

REST and MCP use the same operations service. Specialized reads include contacts lookup, Health, paired iPhones and both status projections. Legacy result serialization follows Go struct field order, including nested read models. Command envelopes, descriptor argument shapes, required versions and wait bounds are checked before forwarding. The Go gateway remains the command/expiry authority; caller expiry and admission-era values are forwarded unchanged so retries retain lookup semantics.

Doctor opens SQLite read-only and checks integrity, journal application, shape violations, recursive private modes, versions, storage encryption and authenticated four-timestamp clock samples. Status is read-only. CLI retention durations use Go units; SIGINT/SIGTERM initiate graceful shutdown with a ten-second drain bound.

Query blob grants use the frozen SQLite tables, are principal/scope/expiry checked, and authorize verified cached blob reads. Uncached query blobs are fetched in bounded signed chunks, digest verified and persisted; range responses are supported.

## Validation on Ubuntu

- `make rust-ci` and lab-feature workspace Clippy have passed during integration; final receipts are collected at the frozen implementation commit.
- The product-adapter frontdoor scenario passes 30 checks: all four unchanged MCP exchanges in JSON and SSE, external authorization/limits, legacy initialization and cancellation notifications. Its unauthenticated golden route is a lab-only internal-handler seam matching the Go golden fixture. Production routes remain authenticated.
- `go run ./lab/replica-parity <Rust binary>` checks sixteen REST and sixteen MCP reads on synthetic Go/Rust replicas, actual command forwarding with independently inspected gateway ledger disposition, and active REST/MCP cancellation through the real SyntheticApple peer. It passes. The user authorized checking heartbeat timestamps, staleness, local replica observation time and read-model timing separately; deterministic payload bytes, including the MCP text payload's field order, must match.
- Mixed Go/Rust convergence has passed with identical snapshot digests.
- The latest completed full matrix before freezing passed 24/25 runnable scenarios. CORE-GATEWAY-DIALS had a lost-response MCP retry failure; its focused scenario and a direct REST/MCP recovery reproduction passed. A fresh immutable-binary full matrix is required before merge. Existing scenarios and frozen corpora have not been edited.
- The requested thirteen native scenarios remain **pending Air real run**, unrun. The current harness reports twenty-two blocked cases overall, including additional native/remote/relay cases; blocked cases are not executed.

## Commands

Production binary (on Ubuntu):

```
CARGO_TARGET_DIR=$HOME/ab-cargo-target cargo build --manifest-path rust/Cargo.toml --bin bridge
```

Harness binary adds `--features ab-runtime/lab-queries`. `make replica-matrix` selects that feature automatically. Set `AB_LAB_ROOT` and `TMPDIR` to directories on the Ubuntu home filesystem when `/tmp` quotas prevent staging executables. This override preserves the default harness root and all assertions. Go 1.27.3 was installed from the official archive under `~/go-sdk`; put `~/go-sdk/bin` and `~/.cargo/bin` on PATH.
