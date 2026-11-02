# Phase 3 interfaces

## Worker A: ab-state

`ab-state` depends only on domain/wire and SQLite. Synchronous methods should run on a blocking thread. `State<Replica>` is Send (not Sync); callers may share it through a mutex. No Gateway write API is exposed yet.

```rust
use ab_state::{State, Replica, Visibility, Cursor, ReadQuery};
let mut state = State::<Replica>::open(home)?;
let cursor = state.head()?;
let install = state.begin_install(&begin_feed_frame, epoch, revision)?;
state.snapshot_page(&install, &page_feed_frame)?;
let cursor = state.install_snapshot(install)?;
let cursor = state.apply_batch(&wire_batch)?;
state.discard_capture()?;
let result = state.read(&visibility, &query)?;
let mut changes = state.changes(visibility, cursor)?;
let batch = state.next_changes(&mut changes)?;
```

`Cursor { epoch: String, subscription_revision: u64, sequence: u64 }` uses Go field names on serialization. `Visibility` is `HashMap<ab_domain::Source, bool>`. `ReadQuery` mirrors Go: `collection, id, text, after: String`, `include_deleted, include_aliases: bool`, `limit: usize`. `ReadResult { records: Vec<Record>, cursor: Cursor, next: String, aliases: Vec<Alias> }` uses Go omit-empty rules. `Batch` and `FeedFrame` are `ab_wire::messages` types. `Change` is a typed sequence/record/op wrapper. `SnapshotInstall` is an opaque staging token. `next_changes` returns `Option<Batch>`; None means caught up (caller provides cancellation/wakeup/polling). `Error` has SQLite/IO/JSON/wire variants and `Contract(ab_domain::ErrorCode)` for reset/conflict/corrupt/invalid/not_found.

Read models use `state.read_model(&visibility, operation: &str, query: &serde_json::Value) -> Result<serde_json::Value, Error>` with operation names `messages.chats`, `messages.thread`, `messages.search`, `messages.collection`, `messages.handles`, `reminders.lists`, `reminders.items`, `calendar.agenda`. JSON shapes follow Go contracts. This keeps new shared-domain definitions out of other workers' scope.

`digest(&[Record]) -> String` hashes sorted `collection\0id\0version\n` (Go snapshot digest). `retention_floor() -> Result<u64, Error>` reads the durable floor. Record visibility includes system-source projection and removes contact links when Contacts is hidden.
