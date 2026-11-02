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

`Cursor { epoch: String, subscription_revision: u64, sequence: u64 }` uses Go field names on serialization. `Visibility` is `HashMap<String, bool>` (source names plus `commands:all` and `principal:<id>` keys, matching Go). `ReadQuery` mirrors Go: `collection, id, text, after: String`, `include_deleted, include_aliases: bool`, `limit: usize`. `ReadResult { records: Vec<Record>, cursor: Cursor, next: String, aliases: Vec<Alias> }` uses Go omit-empty rules. `Batch` and `FeedFrame` are `ab_wire::messages` types. `Change` is a typed sequence/record/op wrapper. `SnapshotInstall` is an opaque staging token. `next_changes` returns `Option<Batch>`; None means caught up (caller provides cancellation/wakeup/polling). `Error` has SQLite/IO/JSON/wire variants and `Contract(ab_domain::ErrorCode)` for reset/conflict/corrupt/invalid/not_found.

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
