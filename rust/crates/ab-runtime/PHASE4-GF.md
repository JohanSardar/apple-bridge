# G-F gateway assembly: incremental lifecycle slice

This change is incomplete gateway assembly. It adds Helper configuration and cancellable Go-compatible launch supervision, atomic private runtime configuration, locked offline restore and backup, remote backend selection commands, direct replica pairing, HTTPS iPhone pairing code admission, agent create/list/revoke, and principal revocation through Trust.

Ubuntu `make rust-ci` passes, including lint, workspace tests, formatting, Clippy, cargo-deny and golden corpora. The independent isolated-home maintenance scenario passes eight checks, including cross-language singleton locking and credential reopening. Helper argument construction is implemented but no Helper launch was attempted.

`make gateway-matrix` reports **0 pass, 0 fail, 198 blocked** for the default Linux selection. Each is blocked before execution by gateway startup/IPC readiness. The complete IDs and per-scenario harness blockers are in `evidence/phase4/gateway-matrix.json`. The other **62 default Darwin scenarios were not run**. The matrix exits nonzero as expected for blocked qualification; this is not a passing acceptance gate.

Remaining assembly requires G-B's ApplePeer/local commands, Service's Local target and a Gateway read-store adapter; web approval wiring; feed/public listeners; Helper BlobRead; concurrent ingress reconciliation; phone ledger/approval adapters and atomic health-stream capture. `rust/INTERFACES.md` records the owner-facing contracts. `pair --via-remote` for replicas, `web enroll`, and `upgrade` remain incomplete. Phone pairing prints the code but not a QR image. APP-*-SIM, real smoke, and 24-hour shadow replay are unqualified. No live gateway, real scenario or permission prompt was touched.

Build on Ubuntu:

```sh
cargo build --manifest-path rust/Cargo.toml -p ab-runtime --features lab-harness --bin bridge
```

Reproduce isolated checks with the Rust binary and a lab-tagged Go bridge:

```sh
python3 rust/crates/ab-runtime/scenarios/maintenance.py /path/to/rust/bridge /path/to/go/bridge
```

## Gateway startup and command assembly continuation

The Rust gateway now starts IPC, API and pinned TLS feed listeners against one shared State<Gateway>. It wires local reads/commands, query-blob hydration, Helper BlobRead and background capture hydration, feed/direct forwarding, Web/owner approvals, retention/status/GC and online backup through a private control socket. Shutdown preserves in-flight preparation for expiry recovery. Native action conversion is currently limited to Reminders pending G-B3.

Fresh Ubuntu make rust-ci passes, including lint and golden corpora. The isolated binary startup scenario passes capture/read/restart/deduplication/fence/shutdown checks. All nine core.commands scenarios pass with gateway=rust, including version conflicts, idempotency, expiry, crash ambiguity and restore/admission era.

The first startup-enabled clean Linux matrix completed with 14 pass, 105 fail and 79 blocked; that run preceded the command/forwarding fixes. The fresh build-host matrix completed all 198 Linux scenarios: **28 pass, 91 fail, 79 blocked**. The committed gateway-matrix.json replaces the old startup-blocked evidence. No Darwin or APP qualification is claimed. The Air has less than 3 GiB free, below the Phase 4 build budget. No live gateway/home, real scenario or permission prompt was touched. Build artifacts are excluded from the commit.
