# Gateway ApplePeer and local command foundation

The server accepts the unchanged Go SimApplePeer over ipc/1. It binds an owner-only Unix socket, checks peer UID, authenticates fresh HMAC nonces, allocates an owner fence, replaces the active session, bounds mutation bytes and separates query responses from capture commits. Capture receipts are installed before Committed and before queued command replies. A cancellation token prevents cleanup from an earlier phase from deleting the next phase's waiter for the same command ID.

The local engine in ab-operations owns admission, deduplication, approval, preparation, durable dispatch transition, outcome correlation, bounded delayed-capture Resolve, per-part strengthening, explicit resolution and recovery without redispatch. Helper supervision preserves the Go open launch chain. No real Helper was launched for these tests.

## Reproduce on the build host

Use an isolated checkout, TMPDIR/GOTMPDIR and CARGO_TARGET_DIR on `ssh build`, with Cargo and `~/go-sdk/bin` on PATH and `RUSTC_WRAPPER=$HOME/.cargo/bin/sccache`.

```sh
cargo build --manifest-path rust/Cargo.toml -p ab-applepeer --example ipc_probe --features lab-harness
cp "$CARGO_TARGET_DIR/debug/examples/ipc_probe" "$TMPDIR/gb-ipc-probe"
go build -tags lab -o "$TMPDIR/gb-go-peer" ./rust/crates/ab-applepeer/probe
go build -tags lab -o "$TMPDIR/gb-lab" ./cmd/bridge-lab
python3 rust/crates/ab-applepeer/probe/matrix.py "$TMPDIR/gb-go-peer" "$TMPDIR/gb-ipc-probe" "$TMPDIR/gb-lab"
python3 rust/crates/ab-applepeer/probe/query_matrix.py "$TMPDIR/gb-go-peer" "$TMPDIR/gb-ipc-probe" "$TMPDIR/gb-lab"
make rust-ci
```

Copy lab-feature executables outside Cargo target before running probes. Workspace tests can rebuild the example without lab features.

The paired processes use the real Go SimApplePeer, SQLite ledgers and capture state. The Rust identity/action adapters are synthetic fixtures. The matrix compares wire and lifecycle dispositions, normalized evidence, effect counts and observed-ID presence. Generated native keys, checkpoints, tickets, expiry and query blob IDs are normalized; other evidence fields must match. It checks stale-fence rejection, lost outcomes, ambiguity, UI interference, delayed captures, approval/rejection/expiry, capture artifacts and missing upload support.

Retries after a stored terminal expiry and after isolated admission-era rotation return stored status. New IDs with stale eras are rejected; changed valid requests reuse no ID. Recovery and explicit Resolve leave both effect counts and dispatch-guard start counts unchanged. The actual gateway CORE-CMD-ERA scenario independently exercises restore. These probes do not compare database/history bytes; shadow replay owns that gate.

The query probe compares result shapes, explicit null, input validation, mail rewriting, document file authorization, large inline staging/cleanup, scoped query grants, expiry, capacity, digest checks, hydration, audit counters and deadlines. A saved blob reader from an old Helper session fails unavailable after reconnect, while the new session remains readable. Deadline messages are normalized to the deadline code because Go races its transport deadline against the Helper deadline reply. Only the generated staged-file path's audit byte count is normalized, after checking its bounded nonzero size. A separate Rust burst verifies 100 admissions followed by two capacity rejections within one second.

The lifecycle canonicalizes resolved actions using G-B3's ab-commands encoder. A codec test compares 540 NativeAction and complete Prepare payloads against the Go-generated corpus. Production runtime adapters and upload staging on main remain in use. Query/session/audit contracts and cross-owner integration requirements are in rust/interfaces/ab-operations.md and rust/interfaces/ab-applepeer.md.

A deterministic race test verifies that queued expected versions are checked after the preceding object's lock is released. No live home, real Helper, real-level scenario or permission prompt is used.
