# Worker C implementation checkpoint

This is an integration checkpoint, not a Phase 3 exit-gate pass. No live home or gateway was used. Builds and scenarios ran on Ubuntu in `/home/johan/ab-phase3-c` against copies of workers A/B's draft crates.

Implemented: descriptor-backed REST and rmcp 3.8 tools, loopback serving, bearer authentication including health, exact Host/Origin lists, 1 MiB input limit, per-principal rate windows, redacted audit sink, Go error status mapping, SSE changes, concrete State read and Trust identity adapters, signed direct mTLS command forwarding, same-ID status recovery after unavailable submission, lookup-only handling, 15-second read-your-write wait, optional generated command IDs, forwarded wait_ms, private config/audit/lock files, join (including listen), token creation and revocation CLI entry points.

Current default build intentionally refuses `start`: `replica-feed` integration is gated until State provides `restrict(&Visibility)` and `blob_references()`. The adapter is written against those requested interfaces in `rust/INTERFACES.md`; it is not verified. Do not present this binary as a functioning replica yet. The draft feed crate also lacked command request exchange when inspected. Gateway-dialed command forwarding therefore remains unavailable; no fallback to direct HTTP is made for that route.

Remaining worker C work:

- Integrate and verify the feed State adapter, then enable replica-feed by default. Acquire the home singleton lock before opening State or starting the feed, and persist actual listen addresses.
- Connect the authenticated feed command exchange to GatewayTransport, including session cancellation and pending request cleanup.
- Complete replica.status and sources.status projection, all specialized reads, contacts lookup, native query forwarding and convenience blob reads. State currently owns only a subset of read models.
- Match command validation and expiry rules fully. Current command validation covers IDs, operation envelope, scope and wait_ms; the Go gateway remains authoritative for arguments.
- Complete doctor (integrity, file modes recursively, versions, encryption and clock) and status without writable inspection. Current doctor explicitly reports incomplete checks.
- Match remaining CLI flags and signal shutdown behavior. Current runtime only handles Ctrl-C gracefully.
- Run full MCP golden authorization/cancellation/legacy coverage through the product adapter. The frozen fourth exchange calls an unauthenticated internal Go handler; the outer Rust router requires bearer authentication. It has not been replaced or weakened.
- Prove byte-for-byte reads against a Go replica on identical synthetic data and a command against a Go gateway. Neither has been proven.

Fresh Ubuntu evidence:

- `cargo clippy --manifest-path rust/Cargo.toml -p ab-runtime -p ab-frontdoor -p ab-operations --all-targets --no-deps -- -D warnings`: pass with default features. Dependency crates are excluded from this Clippy claim.
- `go run ./tools/lint .`: pass.
- `cargo test --manifest-path rust/Cargo.toml -p ab-wire -p ab-catalog --locked`: 15 integration tests pass.
- Build `frontdoor_fixture`, then run `python3 rust/crates/ab-runtime/scenarios/frontdoor.py rust/target/debug/examples/frontdoor_fixture`: 24 synthetic checks pass across JSON and SSE, including three golden exchanges per mode. These use typed mock Identity/State/Transport; they do not establish State read parity or real forwarding.

The parent integration must regenerate Cargo.lock with the final A/B manifests and run full rust-ci. No dependency graph or existing scenario was weakened. No PR was created because the requested gates are incomplete.
