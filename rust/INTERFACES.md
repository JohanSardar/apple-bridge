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
## Phase 4 G-C: gateway feed integration

G-A's gateway interfaces are not present at base `ffa94fe2`. G-C exposes `ab_replication::server::GatewayState` as the temporary adapter contract: `head`, `retention_floor`, `freeze_snapshot(visibility, revision)`, `changes(visibility, cursor, capture_atomic)`, and `next_changes(&mut iterator)`. Freeze returns `FrozenSnapshot` (lease `renew`, immutable ID/cursor/count/digest/expiry/records). The iterator must be retained from replay through live; `None` means caught up, `Error::Contract(Reset)` means retention reset. Implement the adapter when G-A lands. Synchronous methods must be bounded and run in a blocking execution context. Trust session validation runs every 100 ms and terminates feeds when revision changes or credentials disappear.

G-B → G-C BlobRead seam needed: a cancellation-aware async read taking Helper handle, offset and bounded length and returning `(bytes, done)`; ownership stays with the active authenticated Helper connection. No ApplePeer dependency is added to Blobs (that would create the frozen graph's operations/applepeer cycle).

G-C → G-A activity persistence request: provide the existing Go `agent_activity` schema through `record_agent_activity`, `agent_activity(principal, operation)`, and 90-day pruning. Preserve read-count increments, sequence-monotone command updates, argument length/hash only, and newest-first limit 1000. G-C cannot write State's SQLite connection through Trust without violating the crate graph.
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

G-A integration detail: `source_state(&mut self, Source, status: &str, Coverage, capabilities: &[String], reason: &str)` publishes the Go source record. `is_capacity(&Error)` classifies SQLite FULL/ENOSPC; `commit_capture` tracks failed sources and clears capacity after successful capture, projected by `read_model("sources.status")`. `set_blob_protection(Arc<dyn BlobProtection>)` installs the G-C adapter; implement `BlobProtection: Debug + Send + Sync` with `delta(&ReferenceSet, committed: bool) -> ab_state::Result<()>`. ReferenceSet maps owner strings to JSON reference arrays. State protects a temporary transaction owner before commit, publishes durable owner deltas after commit, and releases the guard. Snapshot owners remain protected until `close_snapshot`. Adapter errors must preserve the ab-state typed error boundary.

`CommandRecord.upload_references` uses exported `ab_state::BlobReference`; `error` uses `Option<ab_state::CommandError>` (string code, message, optional current Record, retryable), preserving Go struct field order and supporting codes beyond the shared domain enum. These replace untyped objects at the ledger boundary.

## G-D3 delivery integration (gateway owners G-A/G-B/G-E)

`ab_iphone::delivery::Delivery<G: Gateway>` now owns signed fetch/results, token routes, approval routing, encrypted jobs, Live Activity payloads and expiry/revocation sweep. `Gateway` is the synchronous, serialized adapter for the gateway ledger and capture transactions; no alternative ledger or state database is created. Runtime must call `handle(path, request, now)` on a blocking thread, mount POST routes with Go body limits, and schedule `push_once(client, now)` every thirty seconds and on command/token changes. Do not hold a runtime state mutex across `push_once().await`.

G-A/G-B: implement `Gateway::{commands,command,transition,authorize,decide,ingest_health,ingest_locations}`. `transition` must CAS the prior state, retain command fields/history and attach the exact supplied signed receipt evidence. `authorize` must reauthorize current grants and descriptor admission, returning `Unavailable` only for temporary unavailability. `decide` verifies hash and current approval state. Admission remains through the common ledger; do not admit commands through the phone endpoint. Connector approval callbacks are optional.

`ingest_health(device,batch,digest)` must perform the Go `health_streams` sequence/digest check AND record/journal writes in one transaction: same sequence+digest is a no-op, otherwise only prior+1 is accepted; gaps/replays with changed content return Conflict. The digest is Go-compatible encoding of the validated batch. IDs are `hsm_` + SHA256(device + NUL + stream + NUL + sample id); update/delete versions, cursor and journal as Go. Do not implement sequence checking outside the capture transaction. `ingest_locations` must use CommitCapture with generation `iphone-location/1`, source iphone, collection `iphone.location`, native key device + ':' + id; use Apple's timestamp epoch and current generation predecessor. Identical record bytes must dedupe through CommitCapture.

G-E: `Gateway` and `QueuedCommand` are the fixture seam for binary-driven APP scenarios. The runtime baseline still cannot run role=gateway; G-A ledger APIs have now landed in PR #536, but the concrete delivery adapter and transactional health ingestion remain unwired, so these interfaces alone DO NOT qualify APP-*-SIM. Preserve the existing scenarios and their assertions; do not remove `binary_harness_required` until actual binary runtime integration is present. A crate fixture is not a gateway scenario pass.

G-D3 follow-up: `Arc<Delivery<G>>::router()` supplies the POST HTTP routes, exact packet field ordering, Go newline responses and separate 32 KiB / 16 MiB body limits. `Delivery::run(client, stop)` provides the thirty-second/wake push loop; call `wake()` after admission/approval. Its database preparation runs through `spawn_blocking`; HTTP database calls also run on blocking workers. `HealthBatch::advance(prior: Option<&HealthCursor>)` owns sequence and digest validation; invoke it inside the same transaction as record/journal writes. `None` means exact replay; `Some(cursor)` means advance; gaps or changed replay fail with Conflict.

## Phase 4 G-D2: ab-remote

`ab_remote::Ingress::run(&self, Gateway { address }, stop: watch::Receiver<bool>)` runs an independently cancellable ingress. `run_all(&[Box<dyn Ingress>], Gateway, stop)` starts relay and Tailscale together. `Gateway.address` is the loopback public frontdoor listener; runtime must apply public Host/Origin policy there. The adapters do not route replication or change the frozen relay/1 handshake.

`Relay { home, endpoint, zone, client, certificates: Arc<Certificates>, tls: Arc<rustls::ClientConfig> }` uses outbound WSS and smux v2, terminating stream TLS locally. `Certificates::new(home, directory)` reuses Go SEC1 ACME/account and TLS keys and atomically renews `remote/relay-cert.pem`. Runtime supplies its normal trusted roots for both HTTP and WSS. `Tailscale::local(home, socket)` uses the installed app's Unix LocalAPI with ETag CAS; no embedded tsnet. Its target is a random capability path on an ephemeral IPv4 loopback listener.

Runtime wiring requested from its owner: accept `bridge remote on|off|status --via relay|tailscale-app|both`, call `ab_remote::state::command(home, action, via)`, and reconcile the enabled adapters from `remote/ingresses.json`. The additive selection file is authoritative for Rust once present. Without it, use existing `remote/config.json` (`backend=system` means tailscale-app). The legacy config always retains one Go-readable backend for rollback; `relay.json` and `lease.json` retain Go's exact shapes. CLI calls take the same `remote/config.lock` flock as Go. The owned crate includes an isolated `--example bridge` driver exposing these commands and exercising concurrent adapter lifecycles; changing the product bridge binary remains runtime ownership.
## Phase 4 G-E: remaining gateway harness contracts

The gateway matrix uses `bridge start --role gateway --home <isolated-home> --socket <isolated-socket> --peer-addr 127.0.0.1:0 --api-addr 127.0.0.1:0` and records a runtime/IPC startup blocker before qualification when that role is unavailable. Every default Linux SIM entry is included; remaining in-process imports and semantic product helpers are reported conservatively from the selection inventory. This inventory is not proof of runtime reachability.

Binary-driven consent setup now uses `bridge agent list` and `bridge agent create --name <synthetic-name> --level approve --scope messages:write --home <isolated-home>`, returning `{"agent":...,"token":"..."}`. Activity inspection uses SQLite mode=ro rather than State::open. SimApplePeer and IPC responsiveness use a lab-owned authentication/framing client.

Remaining P/D conversions require product-owned CLI or HTTP seams for trusted device session creation and lifecycle, OAuth setup/revocation and metadata validation, delegated identity fault injection, iPhone pairing/ingestion, and blob staging/GC fixtures. Existing assertions must remain intact; reporting these paths blocked does not count as conversion. G-E has not completed those conversions.

Shadow replay currently reproduces captures with fresh IPC authentication and diffs exact persisted cells. It cannot reconstruct command admission from Prepare/Dispatch alone. G-B/G-A integration needs an explicit contract for replaying authenticated command admissions and deterministic bridge ID assignment/time, or a separately approved identity-aware comparison that still verifies original gojson bytes. No production randomness, persisted format, or live runtime should be changed merely to make this tool pass. Current Go-Go raw diffs intentionally expose independently assigned IDs and journal times; they are not Rust parity evidence.

## G-C continuation: concrete gateway adapters and upstream uploads

`ab_replication::gateway::StateAdapter(Arc<std::sync::Mutex<State<Gateway>>>)` implements the feed State contract using G-A's landed methods. It preserves State's retained iterator, maps Reset at the typed boundary, and closes leases when snapshot transfer completes or is cancelled. State operations move into Tokio's blocking context on a multithread runtime; single-thread callers must invoke synchronous adapters from their blocking executor.

Install `State::set_blob_protection(Arc::new(ab_replication::gateway::ProtectionAdapter(blobs.clone())))` before capture/serving. The adapter accepts `Arc<tokio::sync::Mutex<ab_blobs::Store>>`, maps State reference owner deltas to `update_protection`, and releases staging pins only for committed deltas. Install it from a blocking context.

`Server` now has `uploads: Option<Arc<dyn ab_replication::upstream::UploadHandler>>`. `Uploads::new(authority, blobs)` supplies certificate/delegation authorization, expiring session-bound binary prepare grants, bounded bindings and close cleanup. The callback trait allows G-B's operations adapter to retain lifecycle ownership without adding a replication-to-operations dependency. `GatewayTransport::{upload_control,upload_chunk}` use the existing authenticated feed session and pending-response registry; reconnect never replays them.

`ab_blobs::uploads::{UploadRequest,UploadStatus}` preserve Go field names. `Store::{upload,append_upload}` persist the existing upload lease and part-file layout, validate offsets/digests, pin committed bytes and create the message-upload alias. These synchronous methods require a blocking execution context in production.

`Server::dial(stop)` runs gateway-initiated pairing and pinned feed reconnection. `Authority::{dial_pairings_once,dial_replicas,confirm_dial}` expose the corresponding durable Trust steps. G-D's phone-purpose pairings and fields remain preserved.

`ab_trust::activity::{ActivityLog,AgentActivity}` provides synchronous activity persistence against the already initialized State schema through a separate connection. It does not initialize or migrate State and does not replace G-A's transactional command publication. Runtime should open it after State, record redacted read activity and expose the scoped list API. List prunes after 90 days and returns at most 1000 newest rows.
## Phase 4 G-F: runtime assembly integration requirements

Runtime's first slice adds Helper configuration persistence, the Go `open -n -W -g -a` launch argument sequence, cancellable supervision, offline restore under the shared home singleton lock, and remote selection commands through `ab_remote::state::command`. Configuration writes use private atomic replacement. These entry points do not qualify gateway startup or APP scenarios.

Assembly requires G-B's published `LocalGateway` to be reachable through `ab_operations::CommandTarget::Local`, including Service dispatch and frontdoor query-blob routing. The current Service has only an irrefutable Remote variant. These changes belong to the operations/frontdoor owners; runtime cannot build a local Service without them. Please also expose a Gateway read adapter with the same query/blob behavior as `StateReads`, or a generic StateReads implementation supporting Gateway. Runtime must retain one shared `Arc<Mutex<State<Gateway>>>` across ApplePeer, local commands, feed, and phone delivery.

The iPhone health adapter additionally requires a State-owned transaction API combining `HealthBatch::advance` sequence/digest acceptance, `health_streams` persistence and capture record/journal writes. Runtime must not check the sequence separately from CommitCapture or open a second writable SQLite connection. G-A should expose the transaction seam without depending on ab-iphone; validated sequence/digest and record data can cross it as wire/domain types. Missing this seam blocks APP-HEALTH-SIM qualification even after HTTP delivery routes are mounted.

At main `87cf4790`, G-B (#541), G-C (#535) and G-D1 (#539) are unmerged. `make gateway-matrix` on Ubuntu reaches its startup probe and reports all 198 default Linux scenarios blocked before execution because the Rust gateway does not create its ipc/1 socket. The other 62 default scenarios require Darwin and were not run. No blocked scenario is counted as a pass, and no APP qualification is claimed.
