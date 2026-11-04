> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# Phase 4 G-C integration status

PR #535 continues the original gateway feed work without replacing its commits. The continuation merges G-A's gateway State and preserves G-D's Trust/iPhone modules.

Implemented and exercised on isolated synthetic homes:

- TLS 1.3 enrollment and feed authentication, durable pairing attempts, scopes, revocation, signed delegation and agent credentials.
- Snapshot paging from the concrete `State<Gateway>` through `gateway::StateAdapter`, lease renewal and drop cleanup, retained replay/live iterator, resume, scope resnapshot and record-authorized blob download.
- `gateway::ProtectionAdapter` connects State transaction/snapshot reference deltas to the blob store's owner-union protection.
- Gateway-dials pairing and feed reconnection with certificate pinning, proof-bound bootstrap, durable completion, cancellation and bounded active connections.
- Upload control and binary chunks in both Rust client and server. Session-bound prepare grants expire after 30 seconds and are consumed once; at most sixteen bindings per certificate survive. Upload leases, offsets, content digests and pins persist in the Go-compatible blob layout. Commit creates the message-upload alias.
- `ab_trust::activity::ActivityLog` opens the existing State-owned schema without creating or migrating it. A separate SQLite connection records redacted activity and reads newest-first with the Go limit and 90-day pruning. Reads increment counts; command updates reject older sequences. G-A continues to publish command activity in its ledger transition transaction.

The runner seeds State once, closes it, clones its state/ledger databases, then serves equivalent captures through Go and Rust gateways to both Go and Rust replicas. It requires equal nonempty final snapshot digests and inspects each replica's committed `applied_batches` through an independent read-only Python SQLite oracle, requiring exactly `(1,2)`. The third run repeats the Rust gateway journey in reverse-dial mode. All three runs also exercise upload from both replica implementations, blob download, reconnect/resume, scope narrowing and revocation. Go reopens the Rust-written activity rows and verifies read aggregation and rejection of a stale command update.

Reproduce on Ubuntu from the repository root:

```sh
export PATH="$HOME/go-sdk/bin:$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/ab-phase4-gc-target"
export TMPDIR="$HOME/ab-phase4-gc-tmp"
export APPLE_BRIDGE_SECRET_STORE=file
mkdir -p "$TMPDIR"
cargo build --manifest-path rust/Cargo.toml -p ab-replication --examples
go run ./rust/crates/ab-replication/interop/gateway "$CARGO_TARGET_DIR/debug/examples/gateway_fixture" "$CARGO_TARGET_DIR/debug/examples/gateway_client"
make rust-ci
```

The fixture uses no Helper, live gateway, live home, personal data or permission prompts. Retained JSON evidence is synthetic. It qualifies the concrete State/feed integration, not a production gateway binary.

Remaining Phase 4 integration:

- The runtime owner must construct the gateway role, install the State/blob protection adapter and mount feed, pairing, upload and activity entry points. Main's runtime still rejects roles other than replica, so gateway-matrix remains blocked at startup.
- G-B must supply the authenticated Helper BlobRead adapter and capture retry scheduling. The current blob transfer seam is exercised with synthetic bytes.
- The server retains a conservative one-frame credit window. Pipelined credits, priority writer integration, clock protocol, fresh delegation challenge negotiation and general command multiplexing/cancellation remain separate work.
- Agent summaries and OAuth integration remain with G-D.

The oracle covers a seeded synthetic capture journey, not the complete production corpus, 24-hour shadow replay or the Phase 4 exit gate.
