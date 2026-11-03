# Phase 3 interfaces

## Worker A: ab-state

`ab-state` depends only on domain/wire and SQLite. Synchronous methods should run on a blocking thread. `State<Replica>` is Send (not Sync); callers may share it through a mutex. No Gateway write API is exposed yet.

```rust
use ab_state::{State, Replica, Visibility, Cursor, ReadQuery};
let mut state = State::<Replica>::open(home)?;
let cursor = state.head()?;
let mut install = state.begin_install(&begin_feed_frame, epoch, revision)?;
state.snapshot_page(&mut install, &page_feed_frame)?;
let cursor = state.install_snapshot(install)?;
let cursor = state.apply_batch(&wire_batch)?;
state.discard_capture()?;
let result = state.read(&visibility, &query)?;
let mut changes = state.changes(visibility, cursor)?;
let batch = state.next_changes(&mut changes)?;
```

`Cursor { epoch: String, subscription_revision: u64, sequence: u64 }` uses Go field names on serialization. `Visibility` is `HashMap<String, bool>` (source names plus `commands:all` and `principal:<id>` keys, matching Go). `ReadQuery` mirrors Go: `collection, id, text, after: String`, `include_deleted, include_aliases: bool`, `limit: usize`. `ReadResult { records: Vec<Record>, cursor: Cursor, next: String, aliases: Vec<Alias> }` uses Go omit-empty rules. `Batch` and `FeedFrame` are `ab_wire::messages` types. `Change` is a typed sequence/record/op wrapper. `SnapshotInstall` is an opaque staging token. `changes` is the Go observer iterator: oversized partial captures return Reset; `feed_changes` explicitly permits partial frames. `next_changes` returns `Option<Batch>`; None means caught up (caller provides cancellation/wakeup/polling). `Error` has SQLite/IO/JSON/wire variants and `Contract(ab_domain::ErrorCode)` for reset/conflict/corrupt/invalid/not_found.

Read models use `state.read_model(&visibility, operation: &str, query: &serde_json::Value) -> Result<serde_json::Value, Error>` with operation names `messages.chats`, `messages.thread`, `messages.search`, `messages.collection`, `messages.handles`, `reminders.lists`, `reminders.items`, `calendar.agenda`. JSON shapes follow Go contracts. This keeps new shared-domain definitions out of other workers' scope.

`digest(&[Record]) -> String` hashes sorted `collection\0id\0version\n` (Go snapshot digest). `retention_floor() -> Result<u64, Error>` reads the durable floor. Record visibility includes system-source projection and removes contact links when Contacts is hidden.

## Worker C integration request

Runtime's ab-replication::ReplicaState adapter needs `State<Replica>::restrict(&mut self, visibility: &Visibility) -> Result<()>` and `blob_references(&self) -> Result<Vec<serde_json::Value>>`. Please implement both: first removes records outside incoming scope durably; second returns reference JSON objects usable as ab_blobs::Reference. C's runtime adapter already calls these interfaces. Also `Visibility` actual string keys confirmed; runtime maps Trust visibility including principal identities and commands:all.

`restrict(&mut self, visibility: &Visibility) -> Result<()>` durably removes records, stale projections and blob references outside the replica scope, discards pending staging and invalidates snapshot tokens. `blob_references(&self) -> Result<Vec<serde_json::Value>>` returns persisted blob-reference payloads, including staging protection and explicit reference sets.

## Worker C: ab-operations, ab-frontdoor, ab-runtime

`ab-operations` defines object-safe `ReadStore`, `Identity`, `GatewayTransport`, and `Codec` adapters until concrete A/B integrations land. All adapter errors are the typed `ab_operations::Error`; contract codes map at the frontdoor once. `ReadStore` returns `RawValue` to retain numeric tokens and ordered legacy result fields. `Visibility` has string keys, including `principal:<id>` and `commands:all`; admin must expand to all known sources at the State boundary. `GatewayRoute::Direct` requires pinned mutual TLS; `GatewayRoute::Feed` requires an authenticated feed exchange. The transport must bound responses, decode Go error envelopes, and retain lookup-only metadata. `Codec::json` preserves legacy struct field order while escaping like Go; command hashing uses the frozen capitalized CommandHash entry point. Non-submit forwarding signatures hash the ordered forwarded struct, including its zero-valued command.

`Service::invoke` owns operation meaning, `CommandTarget::Remote` owns command forwarding. `Frontdoor::router` takes an Arc<Service>, explicit hosts/origins/public mode, and an Audit sink. Runtime supplies GoJson, a private AuditLog, the Go-compatible home singleton lock, config persistence and loopback serving. Concrete trust/state/feed adapters belong in the owned crates once dependencies are available.

C integration requests for A: `State<Replica>::restrict(&mut self, visibility: &Visibility) -> Result<()>` must durably remove records outside the new replica scope before serving the new feed; `blob_references(&self) -> Result<Vec<serde_json::Value>>` exposes stored reference payloads without depending on ab-blobs. Runtime's ReplicaState adapter calls both. These methods are required to finish the feed wiring.

C integration request for B: expose a cancellation-aware authenticated `Client::exchange(delegation: String, request: Vec<u8>)` returning the Go `CommandResponse` result/error/lookup_only envelope. It must share the active feed writer, assign a correlation ID, dispatch CommandResponse by that ID and cancel pending requests on disconnect. Remote GatewayRoute::Feed must use this entry point rather than direct HTTPS. Until available, that route returns unavailable.

### A follow-up for C

`restrict` and `blob_references` are implemented as requested. The immutable read-model dispatcher now additionally handles `contacts.lookup`, `contacts.list`, `notes.list`, `health.summary`, `calendar.range`, `sources.status`, and `replica.status` with Go query/result shapes. Route `contacts.lookup`, `sources.status`, and `replica.status` through `read_model` explicitly: the existing C Service routes these names through ordinary `ReadQuery`, which cannot carry contact lookup fields or dynamic status.

Runtime may call `state.set_replica_status(status: serde_json::Value) -> Result<()>` with its current Go-shaped replica status. State filters the status's blob entries and recomputes bytes/backlog/resume/corruption totals through caller Visibility. Until runtime supplies status, `replica.status` returns the Go empty local-status projection. `sources.status` adds Go's source-grants projection; runtime permission status remains runtime-owned.

## Worker B: trust, blobs and feed

`ab-trust::Authority::open(home)` uses the Go file-secret layout. `join(code).await`, `enrollment()`, `create_token(grants, remote)`, `authenticate_token(token)`, `client_tls(pin)`, `server_tls(pin)` and `sign_delegation(principal, command_id, request_hash, challenge)` provide replica identity operations. `Grant::parse` validates grants. `Principal::visibility()` returns Trust Visibility; adapters translate sources, principal IDs and commands:all into the State visibility map.

`ab-replication::Client<S: ReplicaState>` accepts `Arc<tokio::sync::Mutex<S>>`, `Arc<Authority>` and optional `Arc<Mutex<ab_blobs::Store>>`. The typed `ReplicaState` seam mirrors Worker A's synchronous contract, with associated snapshot token, typed Cursor, restrict and blob_references methods. `follow(stop_receiver).await` reconnects, `connect().await` runs one outbound session, `listen(address, stop_receiver).await` accepts gateway-dials feeds. `status()` returns a watch receiver. State errors map to replication Error::State at this adapter boundary. The adapter must enforce snapshot count/digest and ID/ordinal validation before install, and atomic batch/cursor changes. Feed never acks an uncommitted cursor.

`ab-blobs::Store::open(home, cap)`, `begin(digest,size)`, `chunk(digest,offset,bytes)`, `finish(digest)` implement durable resumable reception. `Digest::try_from(String)` validates content hashes. `RecordGrant::from_record(visibility, record)` requires a visible, nondeleted record and authorizes only referenced content. `open_authorized`, `protect` and `gc` apply access and retention rules. All synchronous store calls need a blocking execution context when wired to a production Tokio runtime.

### B → C authenticated GatewayTransport integration

`Client::gateway_transport()` returns a cloneable `ab_replication::GatewayTransport` sharing the active authenticated session. `connected()` becomes true only after validated Welcome. `Client::exchange(delegation: String, request: Vec<u8>).await` is the equivalent feed-only entry point. The handle's `exchange(&GatewayRoute, delegation, request).await` supports `GatewayRoute::Feed` and `GatewayRoute::Direct { address, fingerprint }`.

The result is `CommandResponse { result: Option<Box<RawValue>>, error: Option<ab_trust::domain::ContractError>, lookup_only: bool }`. C's local `ab_operations::GatewayTransport` wrapper maps its route enum to B's, returns the typed response.error through the operations boundary, or constructs `ForwardReply { result, lookup_only }`. B cannot implement the operations trait directly because the frozen dependency graph forbids replication → operations. Keep that thin implementation in C's runtime transport module; no C WIP merge is needed for B's PR.

Feed exchange registers its unique correlation ID before writing CommandRequest in the live lane. CommandResponse resolves that ID without consuming feed credits. At most sixteen requests are pending. Dropping/timing out an exchange removes its pending sender and queues CommandCancel. Dropping the session clears the active handle and every pending sender, so waiting callers fail immediately; reconnect never replays commands. An authenticated relay session entering Client::consume uses the same mechanism. Relay establishment itself remains outside this client transport implementation.

Direct exchange POSTs the same ordered signed body to `https://<gateway>/v1/commands`, supplies `X-Apple-Bridge-Delegation`, uses the enrolled certificate/key plus pinned gateway DER with TLS 1.3, disables redirects, bounds the response, decodes Go error envelopes, and preserves `X-Apple-Bridge-Command-Lookup-Only`. Both direct and feed exchange require a connected authenticated feed, matching Go's disconnected fail-fast behavior. Query timeouts follow native_query.deadline_ms (1–60000 ms, default 1000 ms); ordinary command forwarding defaults to 20 seconds.

Authority exposes `principal(id)`, `authorize(principal, grant)`, `parse_grants(scope)` and `Visibility::to_map()` for C's identity/state adapters. `domain` and `wire` reexports let downstream crates name the public contract types while retaining the frozen direct dependency boundaries. Delegation protocol `delegation/1` negotiates fresh local-principal challenge signing over the active session.

C's adapter can use the following implementation without adding an operations dependency to B:

```rust
#[derive(Debug)]
struct ReplicaGatewayTransport(ab_replication::GatewayTransport);
impl ab_operations::GatewayTransport for ReplicaGatewayTransport {
    fn connected(&self) -> bool { self.0.connected() }
    fn exchange<'a>(
        &'a self, route: &'a ab_operations::GatewayRoute,
        delegation: String, body: Vec<u8>,
    ) -> ab_operations::Task<'a, ab_operations::ForwardReply> {
        Box::pin(async move {
            let route = match route {
                ab_operations::GatewayRoute::Feed => ab_replication::GatewayRoute::Feed,
                ab_operations::GatewayRoute::Direct { address, fingerprint } =>
                    ab_replication::GatewayRoute::Direct {
                        address: address.clone(), fingerprint: fingerprint.clone(),
                    },
            };
            let response = self.0.exchange(&route, delegation, body).await
                .map_err(|_| ab_operations::Error::from(ab_operations::Code::Unavailable))?;
            if let Some(error) = response.error {
                return Err(serde_json::to_vec(&error).ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_else(|| ab_operations::Code::Corrupt.into()));
            }
            Ok(ab_operations::ForwardReply {
                result: response.result.ok_or(ab_operations::Code::Corrupt)?,
                lookup_only: response.lookup_only,
            })
        })
    }
}
```

Dropping C's boxed exchange future cancels the pending B exchange. `GatewayTransport` is independent of `S`, so it can be stored as `Arc<dyn ab_operations::GatewayTransport>` while the feed loop owns `Client<S>`.

### Landed State integration and capture atomicity

B now implements `ReplicaState` directly for `ab_state::State<ab_state::Replica>`. Runtime can pass that concrete state into Client without maintaining a duplicate adapter. The Go/SyntheticApple harness probe now uses this SQLite implementation; evidence marks `state=sqlite`.

Hello advertises `capture_atomic`. Negotiated partial batches enter State staging and replenish only byte credit. Only the final batch advances the acknowledged cursor. Session cleanup discards uncommitted capture parts. FollowStatus.partial_batches counts received partial batches; the interop runner requires at least one actual partial capture and matching final Go/Rust snapshot digests. The runner also requires the feed/direct command exchange success marker, digest-verified blob completion with persisted resumed bytes, and a higher subscription revision with messages removed after scope narrowing.

## Worker D

The selected replica binary must expose the test-build-only command `lab-credential-lifecycle --home <isolated-directory>`. It executes create, get, overwrite, get, delete and get-not-found through its own credential store, using two independently random 32-byte values. It prints one JSON object with `backend` (string) and `created`, `used`, `rotated`, `revoked` (booleans). It must compare retrieved bytes, verify rotation differs from the first value and require the typed not-found error after deletion. Errors exit nonzero. It must delete the test key on every exit and never print the values. This command qualifies CORE-CRED-LIFECYCLE without linking the Go SecretStore into the lab verdict path. The Go implementation is in `cmd/bridge/credentials_lab.go`; production builds omit it.

## Phase 4 G-D: iPhone authentication and delivery integration

`ab_trust::iphone::{PairRequest, Request, Device}` retain the Go JSON field names and padded standard-base64 byte fields. `Authority::pair_iphone_code(address, remote, ttl)` creates the existing `ab1-` phone-purpose code and durable hashed-secret pairing entry. `pair_iphone(&PairRequest)` validates self-signature, purpose, expiry, attempt ceiling and HMAC, then consumes the code and installs the phone in one credential-lock update. Existing Go-created codes/homes work without migration. These methods live in the G-D-owned `ab-trust/src/iphone.rs`; G-C should preserve that module when editing Trust.

Runtime/frontdoor should mount POST `/v1/iphone/pair` (8192-byte body cap) through `pair_iphone`, and `/v1/iphone/status` (8192 bytes) through `verify_iphone` followed by `iphone_gateway_key`. Other phone delivery routes must verify their exact URL path before interpreting the decoded payload. `verify_iphone(path, &Request)` reloads the current active principal, enforces the signed ±60-second timestamp window and returns `(Principal, Vec<u8>)`. `seal_iphone(&Principal, plaintext)` reloads the principal and creates a fresh encrypted/signed `ab_wire::crypto::Packet`; private signing material stays inside Trust. `set_iphone_push_token`, `set_iphone_live_token` and `iphones` retain Go credential storage. The signed-request format intentionally has the existing replay window, without an invented nonce protocol.

`ab_iphone::PushClient::new(endpoint, hostname, credential)` creates an independent HTTPS APNs-relay client with a ten-second timeout and redirects disabled. `push(token, packet, expires)` posts the Go-shaped request to `/iphone/push` and accepts only 204. It has no shared remote-backend selection or shutdown state; runtime can retain it while both relay and Tailscale ingress run. This does not yet implement those ingress adapters.

Pending G-A/G-B interfaces for full iPhone delivery: command enumeration and lookup; compare-and-set command transitions preserving signed evidence; current authorization; command approval decision; query-blob grants/staging; location and health record ingestion through the gateway transaction. Follow the existing Go `internal/iphone/{delivery,capture,location,live}.go` semantics, including dispatched expiry → indeterminate, no command redispatch for live activities and conservative result strengthening. These adapters and runtime HTTP wiring are not implemented by the authentication/transport slice.

Ubuntu verification for this slice: `make rust-ci` passed, including lint, workspace tests, formatting, Clippy, cargo-deny and Go golden corpora. The isolated process-driven interop runner passed 76 assertions with zero failures across Rust/Go code creation × Rust/Go pairing, competing pairing processes, signed current/stale/future requests, wrong-path signatures, encrypted packet decryption/tampering/device binding, Go reopening Rust-stored push/live tokens, and byte-identical APNs relay request encoding. Rust packet-byte equivalence is covered by the existing `ab-wire` frozen iPhone codec test. The APNs HTTPS request has not yet been exercised against a local relay fixture.

Reproduce from an isolated checkout on Ubuntu (set PATH for Cargo and Go first):

```sh
cargo build --manifest-path rust/Cargo.toml -p ab-iphone --example interop
go build -o /tmp/ab-gd-iphone-oracle ./rust/crates/ab-iphone/interop
python3 rust/crates/ab-iphone/interop/run.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/interop" /tmp/ab-gd-iphone-oracle
```

This is not APP-PAIR-SIM/APP-CMD-SIM qualification: the runtime on the initial Phase 4 baseline rejects non-replica roles. OAuth/consent, product WebAuthn/advisory resolution, relay/smux ingress, LocalAPI, command delivery and record ingestion remain G-D work. No live gateway, home, passkey or device was accessed.

Direct execution of APP-PAIR-SIM and APP-CMD-SIM with `--impl gateway=rust --rust-binary <Rust bridge>` returned `status=blocked`, `binary_harness_required`: `harness: native iphone.pairing journey requires AppleTruth/Helper; requires isolated binary-driven fixture adapter` (and the equivalent `iphone.delivery` detail). G-E owns replacing these verdict paths; neither was counted as a gateway product pass. The suite performed no device operation.
# Phase 4 interfaces

## G-A: gateway state and ledger

`State<Gateway>::open(home)` opens schema-v2 `state.db` and initializes/checks the separate live `ledger.db`. Missing ledger with prior command evidence fails with `LedgerMissing`. Calls are synchronous and require the caller's blocking executor/mutex, as for Replica.

- `commit_capture(&mut self, &ab_wire::messages::Capture) -> Result<CommitResult>`; result fields `duplicate, after, through, degraded` match Go. Capture alias/reference resolution, records, journal and checkpoint share one transaction.
- `head`, `retention_floor`, `read`, `changes`, `feed_changes`, `next_changes`, `read_model`, `blob_references`, `save_query_blobs`, `query_blob_grant` have the existing replica signatures.
- `next_fence(&mut self) -> Result<u64>`; `checkpoints(&self) -> Result<HashMap<String, ab_wire::messages::Checkpoint>>`; `native_key(&self, collection: &str, id: &str) -> Result<String>`; `observed_record_id(&self, collection: &str, native_key: &str, cursor: &Cursor) -> Result<String>`.
- `freeze_snapshot(&mut self, &Visibility, revision: u64) -> Result<FrozenSnapshot>` returns public `id, cursor, count, digest, expires_at, records`; `renew_snapshot(&mut self, id: &str)`, `close_snapshot(&mut self, id: &str)` manage the ten-minute lease. `prune(&mut self, rows: u64, age: Duration)` honors leases and capture boundaries.
- `admission_era(&self) -> Result<String>`; `command(&self, id: &str) -> Result<CommandRecord>`; `commands(&self) -> Result<Vec<CommandRecord>>`; `pending_approvals(&self) -> Result<Vec<CommandRecord>>`.
- `transition_command(&mut self, from: Option<ab_domain::CommandState>, record: CommandRecord) -> Result<CommandRecord>` and `update_command_parts(&mut self, record: CommandRecord)` implement ledger CAS. Initial admission uses `None`; retries look up `command` first and compare principal/hash. `CommandRecord` and nested `Command`, `CommandEvent`, `CommandPart` are exported by ab-state with Go JSON field names and typed IDs/states. Evidence and operation-specific payloads remain RawValue.
- `backup(&self, path: impl AsRef<Path>) -> Result<()>`; `restore(home, backup, accept_ledger_loss: bool) -> Result<()>` is an offline operation: caller must close the state first. It keeps the live ledger, resets journal and rotates epoch/era.

G-B owns lifecycle evidence validation before requesting an observed transition; G-A additionally enforces the capture commit ordering invariant at persistence. G-C should retain snapshot leases during transfer and close them on all exits. No changes to `ipc/1`, schema v2, or ledger schema are planned.
