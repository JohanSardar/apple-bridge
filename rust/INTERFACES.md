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
                return Err(serde_json::to_value(error).ok()
                    .and_then(|v| serde_json::from_value(v).ok())
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
