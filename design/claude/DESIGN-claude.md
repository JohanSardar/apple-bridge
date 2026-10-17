# Apple Bridge: design draft v0

Status: draft for critique (2026-10-16). Working name "Apple Bridge" (binary names `applebridge-server`, `applebridge`). Author: Claude Opus 5.5. Reviewers: Astra (critique), Fable (tiebreak only).

## 1. Goal

A general product, **not** tied to Toy or any one agent:

- A Mac holds the user's Apple data (iMessage, Reminders, Calendar, Contacts, later Notes, Screen Time, Photos, Health...).
- **Any** other computer can attach a **full, live, 100% synced local copy** of that data, with **read and write** access, set up in a few minutes.
- Any agent (Toy on the Ubuntu laptop, a completely different agent on a different computer, Claude, a script, a phone) uses the same API and MCP surface.

User requirements (Johan, verbatim intent):

1. Sync is **push**, always on and fast. Replicas almost never request data; it's already there.
2. Server ← replica traffic is for **writes**: send iMessages, create/edit reminders, calendar events, everything Apple exposes.
3. No machine except the Mac is special. The Ubuntu server is just one replica.
4. Setting up another agent on another computer later must be easy.

### Non-goals (v1)

- Agent logic, approval policy ("ask before texting friends"), nudging. Those belong to clients such as Toy.
- Multi-user / multi-tenant hosting. One owner, their devices.
- Multiple source Macs merged together (IDs are namespaced so it can come later).
- Bypassing Apple security (TCC, iCloud). Everything runs with permissions the user grants once.

## 2. Architecture

```
┌──────────────── Mac: SERVER (one signed Swift app, LaunchAgent) ─────────┐
│ Sources:  iMessage · Reminders · Calendar · Contacts · (later more)      │
│   each source = watcher (detect change) + reader (full state) + writer   │
│ Store:    SQLite mirror + append-only change log (seq numbers)           │
│ Commands: durable command log, executor, outcome reconciliation          │
│ Exposes:  sync feed (WebSocket) · REST API · MCP · pairing/admin         │
└────────┬───────────────────────────┬───────────────────────────┬─────────┘
         │ push changes              │                           │
         ▼                           ▼                           ▼
   Replica (Ubuntu/Toy)       Replica (any computer)      Thin client (phone,
   ┌────────────────┐         ┌────────────────┐          script): calls the
   │ local SQLite   │         │ local SQLite   │          Mac's REST/MCP
   │ local REST/MCP │         │ local REST/MCP │          directly
   │ writes → Mac   │         │ writes → Mac   │
   └────────────────┘         └────────────────┘
```

Principles:

- **The Mac is the only source of truth.** Replicas never edit their copy. Writes are forwarded to the Mac; the result returns through the normal change feed.
- **The replica is generic.** It mirrors whatever collections the server publishes and knows nothing about Apple. New sources on the server appear in every replica without replica changes.
- **One operation registry.** Every collection schema and every write operation is defined once (JSON Schema). The Swift server, the replica, the REST API, the MCP tools and the docs are generated or validated from it.
- **Same surface everywhere.** The replica exposes the same REST + MCP as the Mac, on localhost. Reads are served locally; writes are proxied to the Mac.

## 3. Data model

Every synced thing is a **record** in a **collection**:

```
record = { collection, id, version, updated_at, deleted, data(JSON) }
```

- `id` is stable and namespaced by server: `<server_id>:<source-native-id>`.
- `version` increments on every change to that record.
- Deletions are tombstones (`deleted = true`), kept so replicas learn about them.

v1 collections:

| Collection | Native source | Notes |
|---|---|---|
| `messages.chat` | chat.db `chat` | 1:1 and group, participants, display name |
| `messages.message` | chat.db `message` | text (decoded from `attributedBody` when `text` is NULL), sender, chat, timestamps, delivered/read, edited/unsent, reactions, reply-to |
| `messages.attachment` | chat.db `attachment` + files | metadata record + content-addressed blob |
| `contacts.contact` | Contacts framework | names, phones, emails, used to resolve iMessage handles |
| `reminders.list` / `reminders.reminder` | EventKit | title, notes, due, priority, completed, list, URL. (EventKit does not expose tags, sections or subtasks.) |
| `calendar.calendar` / `calendar.event` | EventKit | events within a sync window (see §6.3), recurrence rule on the series, attendees (read-only), alarms |

Blobs (attachments, later photos): content-addressed by SHA-256, stored once, referenced from records. Replicas download them eagerly by default (configurable size cap).

## 4. Sync protocol

Transport: one WebSocket per replica over Tailscale (WireGuard-encrypted). JSON messages for v1.

Server state:

- `server_id` (stable) and `epoch` (changes only if the server's store is rebuilt from scratch).
- `change_log(seq INTEGER PRIMARY KEY, collection, record_id, version, op, data)`: append-only, monotonically increasing `seq`.

Flow:

1. **Hello.** Replica sends `{server_id, epoch, cursor}` (its last applied `seq`, or none).
2. **Catch-up.**
   - Same epoch and cursor still inside log retention: the server streams every change after the cursor.
   - Otherwise: the server sends a **snapshot** (all live records per collection, paged) tagged with the `seq` it represents, then streams changes from there.
3. **Live.** The server pushes each new change as soon as it is committed. The replica applies changes in one SQLite transaction per batch and records the new cursor in the same transaction (exactly-once application).
4. **Heartbeats** every 15 s. Missing heartbeats → reconnect with backoff; resume from cursor.
5. **Log retention:** keep 30 days (configurable). Older cursors resnapshot.

Changes are full record states (not field diffs), so applying them is idempotent and order-safe per record (`version` check).

## 5. Writes (commands)

```
command = { command_id (UUID, client-generated, idempotency key),
            op, args, expires_at, client_id }
status  = accepted → executing → succeeded {affected record ids}
                               → failed {error}
                               → expired
                               → unknown {why}   // never auto-retried
```

- The Mac persists the command **before** executing it and marks it `executing` before the side effect. A crash leaves `executing`; on restart it is reconciled (did the side effect happen?), and if that cannot be determined it becomes `unknown`.
- The same `command_id` is never executed twice. Resubmitting returns the existing status.
- **Confirmation by observation:** a write is `succeeded` only when the watcher sees the resulting record (e.g. the outgoing message row in chat.db with no error, the reminder in EventKit). The command result links those record ids.
- **Expiry:** every command has `expires_at` (default 60 s for messages, longer for reminders/calendar). If the Mac was unreachable and the command arrives late, it is `expired`, not run. Texts don't go out 8 hours late.
- **Replica behaviour:** the replica forwards a write immediately. If the Mac is unreachable, it returns `server_unreachable` right away, unless the caller explicitly asked for queueing (with an `expires_at`).
- Command results are also published as records (`system.command`) so every replica sees what happened.

v1 operations: `messages.send` (to handle(s) or chat id; text and attachments), `reminders.create|update|complete|delete`, `reminders.list.create`, `calendar.event.create|update|delete` (with recurrence-span option: this occurrence / future / all), `contacts.create|update` (later).

## 6. Sources (Mac side)

Each source implements: `snapshot()`, `watch(onChange)`, `apply(command)`, `reconcile()`.

### 6.1 iMessage
- Read `~/Library/Messages/chat.db` read-only (needs Full Disk Access). Watch `chat.db` and `chat.db-wal` with a file-system event source; on change, query rows with `ROWID > last` and rows whose `date_edited`/`date_retracted`/delivery fields changed.
- Decode `attributedBody` (typedstream) for message text on newer macOS.
- Send via AppleScript/ScriptingBridge to Messages (`send` to a participant or chat id). Needs the Automation permission. Attachments must be copied into a location Messages can read.
- Confirmation: new `is_from_me` row matching the command in the chat, `error = 0`.

### 6.2 Contacts
- `CNContactStore` with change history (`CNChangeHistoryFetchRequest`) gives real diffs, not just "something changed".

### 6.3 Reminders and Calendar (EventKit)
- `EKEventStoreChanged` says *something* changed, with no diff. The source re-reads and diffs against its own last snapshot (hashes per item).
- Calendar event queries need a date range, so "full mirror" means a **sync window** (default: 2 years back, 2 years forward, sliding daily). Recurring series are stored as one series record plus expanded occurrences inside the window.
- Use `calendarItemExternalIdentifier` + start date for cross-device stable identity, falling back to `calendarItemIdentifier`.

### 6.4 Later sources
- **Notes**: no public API. Read from NoteStore.sqlite (gzipped protobuf) or AppleScript; write via AppleScript. v2.
- **Screen Time**: read-only from knowledgeC.db / Biome streams (with "Share Across Devices", includes iPhone). Undocumented; verify on the target macOS first. v2.
- **Photos**: PhotoKit change observer. v2+.
- **Health**: not on the Mac. An iPhone producer (Health Auto Export, Shortcuts, or a companion app) POSTs to the server's ingest endpoint, and the server republishes it as collections. v2.

## 7. Access, pairing, security

- Network: listens on the Tailscale interface only by default.
- Pairing: `applebridge pair` on a new machine shows a short code; the user approves it in the Mac app's menu (or with `applebridge-server approve <code>`). The replica receives a long-lived key.
- Scopes per client, per collection: `read`, `write`. Example: Toy gets `messages:read, messages:send, reminders:*, calendar:*`.
- Clients can be revoked instantly. Every write is logged with the `client_id`.
- At rest: replicas store an unencrypted SQLite file by default (it's the user's own machine); optional encryption is v2.

## 8. API surfaces (generated from the registry)

Same on the Mac and on every replica:

- **REST**: `GET /v1/{collection}?filter…`, `GET /v1/{collection}/{id}`, `GET /v1/search?q=` (FTS5), `POST /v1/commands`, `GET /v1/commands/{id}`. OpenAPI spec generated.
- **Change stream for apps**: `GET /v1/changes?since=seq` (SSE), so a local agent can react to "new iMessage" in real time.
- **MCP**: tools generated per collection (`search_messages`, `list_reminders`, …) and per operation (`send_imessage`, `create_reminder`, …). Streamable HTTP plus stdio.
- **Direct SQLite**: the replica's schema is documented and stable, so a local agent may read it directly.

## 9. Technology

- **Server:** Swift, one signed app bundle running as a LaunchAgent in the user's login session (TCC permissions attach to the app). SQLite via GRDB; HTTP/WebSocket via SwiftNIO (Hummingbird); MCP via the official Swift SDK.
- **Replica:** Go (single static binary for Linux/macOS/Windows; trivial install) **or** TypeScript (same language as the pi-based agent; best MCP SDK). Recommendation: Go.
- **Registry:** JSON Schema files in the repo; codegen for Swift and the replica language; conformance test suite that every implementation must pass.

## 10. Operational concerns

- Mac must stay logged in, powered and awake (`pmset`, `caffeinate`, prevent sleep on AC).
- macOS updates can revoke or change TCC grants: the server reports per-source health (`ok`, `permission_missing`, `stale`, `error`) as `system.source` records, so every replica can see it.
- A broken watcher must never look like "no new data": source health carries `last_successful_read`.
- Periodic **self-reconcile** on the Mac (full read and diff, e.g. every 10 min and after wake) catches missed change events. No replica involvement.

## 11. Testing

- Each source has a fake implementation (in-memory) behind the same interface.
- Protocol conformance tests: snapshot + resume, epoch change, log retention expiry, duplicate command id, expiry, crash mid-command → unknown.
- End-to-end on a real Mac: send iMessage to self, create/complete a reminder, create/move/delete an event; check every replica converges within N seconds.

## 12. Open questions for the user

- Name.
- Replica language (Go vs TypeScript).
- Just Johan, or also designed for other people to install?
- v1 source set and order.
- Blob policy: download all attachments eagerly, or cap by size?
- Health / Screen Time: route through the Mac server (one hub for all Apple data) or a separate path?
- Is there a latency target (e.g. new iMessage on every replica within 2 s)?
