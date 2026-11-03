# Gateway ApplePeer and local command foundation

The server accepts the unchanged Go SimApplePeer over ipc/1. It binds an owner-only Unix socket, checks peer UID, authenticates fresh HMAC nonces, allocates an owner fence, replaces the active session, bounds mutation bytes and separates query responses from capture commits. Capture receipts are installed before Committed and before queued command replies. A cancellation token prevents cleanup from an earlier phase from deleting the next phase's waiter for the same command ID.

The local engine in ab-operations owns admission, deduplication, approval, preparation, durable dispatch transition, outcome correlation, bounded delayed-capture Resolve, per-part strengthening, explicit resolution and recovery without redispatch. Helper supervision preserves the Go open launch chain. No real Helper was launched for these tests.

## Reproduce on Ubuntu

From the repository root, with Cargo and Go on PATH:

```sh
cargo build --manifest-path rust/Cargo.toml -p ab-applepeer --example ipc_probe --features lab-harness
go build -o /tmp/gb-go-peer ./rust/crates/ab-applepeer/probe
go build -o /tmp/gb-bridge-lab ./cmd/bridge-lab
python3 rust/crates/ab-applepeer/probe/matrix.py /tmp/gb-go-peer rust/target/debug/examples/ipc_probe /tmp/gb-bridge-lab
make rust-ci
```

Use an owned TMPDIR/GOTMPDIR on hosts whose shared /tmp quota is exhausted.

The paired probe runs nine wire cases and nine local lifecycle cases plus a stale-fence rejection. Both processes use the real Go SimApplePeer. Go local commands use the Go SQLite ledger and authority. Rust local commands now use G-A's concrete State<Gateway> capture store and SQLite ledger through GatewayStore. The synthetic action resolver and Rust identity adapter remain test fixtures. Each lifecycle case repeats submission, runs recovery and performs read-only Resolve, requiring unchanged effect counts. Approval checks require zero effects until approval; rejection and expiry require zero effects. The wire cases cover native conflict and UI interference before preparation as well as interference after dispatch. The stale-fence case deliberately preserves a zero fence using an API that exists only under lab-harness; the real Helper counterparty rejects it with zero effects.

Comparisons cover dispositions, states, evidence, resolution, effect counts and presence of observed IDs. Evidence comparisons normalize only generated native_key, capture_checkpoint, ticket and expires_at strings; the remaining JSON fields must match. Bridge record IDs, timestamps, cursors, history and database bytes are not compared. Full multipart attachments and all source-specific validators are not qualified by this probe.

## Integration remaining

This is a foundation PR, not Phase 4 gateway qualification. It does not enable the Rust gateway binary. See rust/INTERFACES.md for the exact seams.

- GatewayStore integrates G-A capture commits, source state, fences, aliases, durable ledger CAS and parts updates. Full database/history byte parity remains the shadow replay gate.
- Runtime must connect Server, LocalGateway, the state/action adapters and Helper supervision. Frontdoor currently destructures the sole Remote variant, so adding Local requires its owner to update query-blob routing too.
- Production ActionResolver adapters must port Go native argument conversion, visible object/container/series resolution, expected-version checks, native conflict capture binding, Notes content restrictions and upload staging. Only the synthetic resolver is supplied here.
- Query transport is available; full local query input rewriting, rate accounting, query-blob hydration/grants and audit still need integration. BlobChunk/BlobUnavailable routing, bounded reads, mail body ID rewriting and query result hydration/grants now exist. Document input rewriting/staging and query audit remain pending. Query protection expiry needs the G-A hook described in rust/interfaces/ab-applepeer.md.
- Source-specific completion evidence validators and capture-blob persistence require their native/blob adapters. The generic correlation engine cannot qualify those operations alone.
- The full --impl gateway=rust scenario matrix remains blocked on the executable wiring. Real smoke and shadow replay remain orchestrator-owned.

No live home, live gateway, real-level scenario or permission prompt was accessed.

## Continuation verification

The matrix additionally reads a 262145-byte SimApplePeer query blob in bounded chunks, verifies its SHA-256 and exact size, checks missing/unavailable/offloaded error mappings, and hydrates a speech synthesis result into SQLite-backed query grants with no returned Helper handle. Receipt lookup uses a separate synchronous mutex so session traffic cannot make committed evidence disappear. A deterministic contention regression test covers this race. These checks still use the synthetic action resolver and do not establish production action conversion, all source validators or admission-era restore behavior.

On quota-constrained Ubuntu hosts, set TMPDIR and GOTMPDIR to an owned directory under the build home for both compilation and scenarios. Use RUSTC_WRAPPER=$HOME/.cargo/bin/sccache when installed.
