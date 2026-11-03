# Phase 4 G-C integration status

This is the first gateway feed journey, using an isolated mock state adapter. It is not the Phase 4 exit gate or a production gateway runtime.

Implemented:

- TLS 1.3 listener with certificate possession and validity checks; enrollment proof endpoint; certificate-authorized feed upgrade.
- Durable one-use pairings, persisted failed attempts, scope revision changes, revoke, certificate and signed delegation verification, agent creation and credential rotation.
- Hello/Welcome, leased snapshot paging, exact byte/cursor acknowledgments, retained iterator through replay/live, resume, Reset, revision invalidation and heartbeats.
- Record-authorized blob download, bounded Helper `BlobRead` adapter transfer, staging, owner-union protection, durable pin release and GC.

The fixture drives the actual Rust feed server and the existing Go replication client and Phase 3 Rust client. It verifies pairing, a nonempty snapshot, live record update, persisted resume, resnapshot after subscription revision changes, transferred blob bytes and identical Go/Rust replica digests. The scenario removes Reminders from both enrolled scopes and asserts that the resnapshot removes the record from both replicas. The projection still needs qualification with the real State adapter. The Go digest oracle is the Go replica snapshot, not a Go gateway serving equivalent captures.

Run on Ubuntu, from the repository root:

```sh
export PATH="$HOME/go-sdk/bin:$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/ab-cargo-target"
cargo build --manifest-path rust/Cargo.toml -p ab-replication --examples
go run ./rust/crates/ab-replication/interop/gateway "$CARGO_TARGET_DIR/debug/examples/gateway_fixture" "$CARGO_TARGET_DIR/debug/examples/gateway_client"
```

Set `APPLE_BRIDGE_SECRET_STORE=file` for the fixture; it uses freshly created temporary homes and no Helper. All output is synthetic. `examples/blob_probe.rs` additionally exercises multiple protection owners, restart, pin release and GC.

Remaining G-C work:

- Replace the mock adapter with G-A's concrete gateway API once published; wire the gateway runtime through its owner.
- Gateway-dials pairing and feed reconnection.
- Upload binary/control direction, command/delegation request multiplexing, cancellation and bounded slots.
- Pipelined credit window and live-priority writer integration; this first server uses a conservative one-frame window.
- Clock protocol and delegation challenge negotiation.
- Pending blob availability notifications and capture retry scheduling with the real Helper adapter.
- Activity persistence through the State owner; agent summaries/OAuth integration with G-D.
- Production gateway binary matrix, concrete State scope projection and independent Go gateway digest oracle.

Do not claim gateway-matrix, upload, reverse-dial, full trust parity or production integration based on this fixture.
