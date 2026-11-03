# G-F Helper capture integration requests

Runtime now adapts Commands::read_blob to ab_blobs::BlobRead and hydrates committed capture handles in a bounded background queue. Locks are released during IPC waits. Pending blobs set refresh_blobs in Open checkpoints; transfers verify size and digest through Store. GC and status share the same store.

G-C follow-up: runtime needs typed Store methods for persisted Helper retry attempts, next_attempt_at, terminal source states (offloaded/missing/failed), and corruption/refetch accounting. Entry.extra and the manifest are private; runtime must not write their files itself. Current runtime retries are bounded in memory and do not qualify gateway retry persistence, exhausted failures or full blob scenario parity. Capture handles should also bind to a stable Helper-session token before being queued; Commands currently binds only individual read requests.
