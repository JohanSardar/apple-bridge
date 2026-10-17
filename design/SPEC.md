# Apple Bridge specification

Status: binding build specification, written 2026-10-17 by the build leader. It integrates `DECISIONS.md` (wins over everything), `debate/RESOLUTION.md`, `debate/MODULES-RESOLUTION.md`, `astra/MODULES-astra.md` as amended, and the 20 findings in `claude/astra-critique-of-claude-r1.md` (cited as **F1–F20**). Background designs are superseded where they conflict. When code and this file disagree, one of them is a bug; fix the wrong one in the same change.

Companion files: [`../GLOSSARY.md`](../GLOSSARY.md), [`../PRINCIPLES.md`](../PRINCIPLES.md), [`../docs/adr/`](../docs/adr/), [`../features.yaml`](../features.yaml), [`PROGRESS.md`](PROGRESS.md).

## 0. Contents

1. Product and guarantees
2. Glossary (summary; canonical in `GLOSSARY.md`)
3. Deployment and processes
4. Repository layout and toolchain
5. Module map and interfaces
6. Contracts: records, collections, operations
7. The Apple seam (ApplePeer IPC)
8. The Source seam (Swift)
9. Sources at full depth
10. State
11. Operations and commands
12. Replication (feed protocol)
13. Blobs
14. Trust
15. Frontdoor (REST + MCP from operation descriptors)
16. Runtime
17. Harness (`bridge-lab`)
18. E2E scenario catalogue
19. Code-level test allowance
20. Quality gates
21. Milestones
22. Known limits

---

## 1. Product and guarantees

Apple Bridge makes everything on a Mac that syncs with the iPhone available on any computer as a live, durable, read/write local copy, through one REST API and one MCP server, identical on every machine.

Sources in v1: **Messages** (iMessage, SMS, RCS; full depth), **Calendar**, **Reminders**, **Contacts**, **Notes**, **Usage** (Screen Time estimates, read-only). Health is advertised `unavailable` (no Mac source; RESOLUTION D5).

### 1.1 Guarantees (what "fully synced" means, F1)

- **Equality within coverage.** Every replica converges to the gateway's bridge state for its authorized scope. Bridge state is equal to Apple's state within each source's declared **coverage** (records, fields, history, attachments, operations), with gaps published as data, never hidden.
- **Messages history** = what is on the Mac. No forced iCloud hydration in v1.
- **Calendar window** = 2 years back to 2 years ahead, sliding daily. Eviction from the window is not deletion.
- **Attachments** download to every replica by default, up to a per-machine cap. Blob completeness is reported separately from metadata completeness.
- **Latency.** While the Mac is awake and healthy: Apple-observable change on the Mac → visible on every connected replica in **< 1 s p95** (capture-to-replica). Apple's own arrival lag (iPhone → Mac via iCloud) and blob completion are measured and reported separately; they are not covered by the target.
- **Writes** are commands with evidence. A command either completes with observed evidence, fails, is rejected before any effect, or is **indeterminate**. Indeterminate effects are never redispatched automatically. No offline queue in v1 (fail fast, RESOLUTION D4).
- **No false deletion.** Absence is published only after a successful, complete, scoped scan. Permission loss, a crash, a decode failure or a window slide never produce deletions.
- **No scope leak.** One visibility policy governs snapshots, changes, queries, search, blobs, commands, command results and MCP.
- **Exactly-once applies to replica application** (each journal change is applied to a replica exactly once, transactionally with its cursor), not to Apple effects.

### 1.2 Non-goals in v1

iPhone app; direct-iCloud connector; private-API injection; disabling SIP; offline command queues; CRDT multi-writer merging; multiple source Macs; Health.

### 1.3 Safety rules for every test and agent (from Johan)

- iMessage tests text only Johan himself (his own number or iCloud email, discovered from his own contact card or Messages). Group tests may include Ben (handle in `~/.config/apple-bridge/test-contacts.env` on the Air, never committed). Never text anyone else.
- Test data is tagged and cleaned up: calendar **"Bridge Test"**, reminders list **"Bridge Test"**, contacts with organization **"Bridge Test"**, notes folder **"Bridge Test"**, message text containing **`[ab-test <id>]`**.
- Never modify or delete Johan's data that tests didn't create. SIP stays on. No security setting changes.
- Never commit personal data (numbers, emails, message text, usage numbers, IPs, tokens). Fixtures are scrubbed with synthetic values that keep binary structure valid.

---

## 2. Glossary summary

The canonical glossary is `GLOSSARY.md` (domain language only). Key terms: **Source, Gateway, Replica, Helper, Bridge record, Collection, Observation, Capture, Source generation, Source checkpoint, Journal, Epoch, Subscription revision, Replication cursor, Snapshot, Command, Admission era, Ticket, Evidence, Disposition, Locally observed, Delivered, Indeterminate, Coverage, Freshness, Principal, Scope, Grant, Pairing, Delegated identity, Blob, Record grant, Reference set, Operation descriptor, Capability, Truth, Scenario, Level, Feature**.

---

## 3. Deployment and processes

```text
┌─────────────────────────── Mac (MacBook Air, always on) ────────────────────────────┐
│  Apple Bridge.app                                                                  │
│   ├─ AppleBridgeHelper (Swift)  ══ ApplePeer IPC (owner-only Unix socket) ══╗       │
│   └─ bridge (Go, role=gateway)  ◄═══════════════════════════════════════════╝       │
│        State · Operations · Replication(server) · Blobs · Trust · Frontdoor         │
│        REST+MCP on 127.0.0.1 (token)      Peer listener on :7443 (mTLS)            │
└───────────────────────────────────────────────┬────────────────────────────────────┘
                                                │ WebSocket feed + command forwarding, mTLS, pinned certs
             ┌──────────────────────────────────┴───────────────────────┐
   bridge (role=replica) on Ubuntu                          bridge (role=replica) anywhere
   State · Operations(RemoteGateway) · Replication(client)  same binary
   · Blobs · Trust · Frontdoor (REST+MCP on 127.0.0.1)
```

- One `bridge` Go binary, two roles. Self-contained per OS/arch (`darwin/arm64`, `linux/amd64`, `linux/arm64`), pure Go (`modernc.org/sqlite`), cross-compiled on the Air.
- The Helper is the only process touching Apple data. The gateway is the only sequencer and command authority. Replicas are equal; Ubuntu is just one replica.
- The Helper and the gateway ship and upgrade together inside one app bundle.

---

## 4. Repository layout and toolchain

```text
go.mod                              module github.com/JohanSardar/apple-bridge, Go 1.27
cmd/bridge/                         Runtime entry (gateway | replica)
cmd/bridge-lab/                     Harness control tool
internal/contracts/                 Records, collections, operation descriptors, wire codecs, validation
internal/applepeer/                 Go side of the ApplePeer IPC (framing, auth, session)
internal/state/                     State
internal/operations/                Operations (ingestion + commands; CommandTarget LocalGateway/RemoteGateway)
internal/replication/               Replication (feed server + client, WebSocket and local SSE)
internal/blobs/                     Blobs
internal/trust/                     Trust (credential stores: keychain, linux secret file)
internal/frontdoor/                 Frontdoor (REST, MCP, OpenAPI from descriptors)
internal/runtime/                   Runtime (role wiring, locks, doctor, restore, upgrade)
native/                             Swift package (macOS 26, Swift 6.3, CLT only)
  Package.swift
  Sources/AppleBridgeHelper/        executable: AppleSession
  Sources/AppleSession/             IPC, fencing, tickets, dispatch guard, UI actor
  Sources/MessagesSource/           Messages
  Sources/AgendaGate/               shared EventKit gate
  Sources/CalendarSource/           Calendar
  Sources/RemindersSource/          Reminders
  Sources/ContactsSource/           Contacts
  Sources/NotesSource/              Notes
  Sources/UsageSource/              Usage (Screen Time)
  Sources/Decoders/                 typedstream, SEGB, Notes protobuf (pure, fixture-tested)
  Tests/DecodersTests/              decoder + time-math tests only
lab/                                Harness (Go packages used by cmd/bridge-lab)
  world/ simapple/ truth/ scenarios/ agent/ featuregraph/ evidence/
  truth/native/                     AppleTruth: separate Swift package, shares no code with native/
  scenarios/<source>/*.yaml         scenario definitions
  fixtures/                         scrubbed decoder fixtures
packaging/                          app bundle, Info.plist, entitlements, SMAppService plist, signing scripts
tools/                              lint (zero comments, import boundaries), featuregraph checker + HTML render
features.yaml                       feature graph
GLOSSARY.md  PRINCIPLES.md  docs/adr/  design/
```

Toolchain on the Air: Go 1.27 (Homebrew), Swift 6.3.3 CLT (no Xcode, so no XCTest UI; Swift Testing via `swift test` works with CLT on macOS 26; if not, decoder tests run as an executable target `DecoderChecks` that exits non-zero on failure). Dependencies (keep minimal, permissive licenses): `modernc.org/sqlite`, `github.com/coder/websocket`, `github.com/modelcontextprotocol/go-sdk`, `gopkg.in/yaml.v3`. Swift: system frameworks only (SQLite3, EventKit, Contacts, ApplicationServices, Security, CryptoKit, Network, ServiceManagement).

All builds, tests and services run on the Air. `make` targets (Makefile at root): `make build` (Go + Swift), `make lint`, `make features` (checker + HTML), `make sim` (sim-level scenario suite), `make real`, `make replica`, `make agent`, `make dist` (cross-compiled replicas into `bin/`).

---

## 5. Module map and interfaces

Vocabulary: Matt Pocock's codebase-design (module, interface, depth, seam, adapter, leverage, locality). One adapter = hypothetical seam; two = real seam. Dependency arrows mean "depends on the interface of".

```text
Control(lab) → Runtime(processes), Frontdoor(HTTP/MCP), Contracts
Runtime → Frontdoor, Operations, Replication, State, Blobs, Trust, ApplePeer
Frontdoor → Operations, Replication, Blobs, Trust, Contracts
Operations → State, Blobs, Trust, ApplePeer, Contracts
Replication → State, Blobs, Trust, Contracts
Blobs → Trust, ApplePeer, Contracts
State → Trust(visibility), Contracts
Trust → Contracts
AppleSession(Swift) → Sources(Swift) → Decoders
```

Import boundaries are enforced by `tools/lint` (§20). Frontdoor never contains semantic validation; Contracts never imports Operations; nothing imports `lab/` except `cmd/bridge-lab`.

### 5.1 Real seams (two adapters each)

| Seam | Adapter A | Adapter B |
|---|---|---|
| ApplePeer (IPC, §7) | SwiftHelper (real Apple) | SimApplePeer (`lab/simapple`, Go, fake Apple with fault knobs) |
| Source (Swift, §8) | Messages, Calendar, Reminders, Contacts, Notes, Usage | (six real adapters) |
| CommandTarget (Go, §11) | LocalGateway (gateway) | RemoteGateway (replica → gateway) |
| Live feed transport (§12) | WebSocket (replica sync) | SSE (local subscribers) |
| Credential store (§14) | Keychain (macOS) | LinuxSecretStore (0600 file under `$XDG_CONFIG_HOME`, or libsecret when present) |
| Frontdoor protocol (§15) | REST | MCP (Streamable HTTP) |
| Truth (§17) | SimTruth | AppleTruth |
| Environment (§17) | SimEnvironment | RealAirEnvironment |
| Actor (§17) | ScriptedClient | CodexMCPAgent |

### 5.2 Interfaces (Go)

Shown as Go signatures; full interface includes invariants and error modes stated beside them. All bounded calls take `context.Context`; cancellation never reverses a dispatched effect. Typed errors (package `contracts`): `ErrUnsupported, ErrUnauthorized, ErrUnavailable, ErrConflict, ErrReset, ErrExpired, ErrCorrupt, ErrCapacity, ErrIndeterminate, ErrInvalid, ErrNotFound`, each carrying a machine code and safe message.

```go
package state
type Store interface {
    CommitCapture(ctx, Capture) (CommitResult, error)
    TransitionCommand(ctx, CommandTransition) (CommandRecord, error)
    Read(ctx, Visibility, ReadQuery) (ReadResult, error)
    FreezeSnapshot(ctx, Visibility, SubscriptionRevision) (FrozenSnapshot, error)
    InstallSnapshot(ctx, SnapshotStream) (Cursor, error)
    ApplyBatch(ctx, Batch) (Cursor, error)
    Changes(ctx, Visibility, after Cursor) (ChangeIterator, error)
    Head() Cursor
}
```

```go
package operations
type Service interface {
    Read(ctx, Principal, Query) (Result, error)
    Submit(ctx, Principal, Command) (CommandStatus, error)
    Status(ctx, Principal, CommandID) (CommandStatus, error)
    Observe(ctx, Principal, after Cursor) (<-chan Change, error)
}
type CommandTarget interface {
    Submit(ctx, DelegatedPrincipal, Command) (CommandStatus, error)
    Status(ctx, DelegatedPrincipal, CommandID) (CommandStatus, error)
}
```

```go
package replication
type Server interface { Serve(ctx, PeerConn, ReplicaIdentity) error }
type Client interface { Follow(ctx, GatewayAddress, Subscription) error; Wait(ctx, Cursor) error; Status() FollowStatus }
```

```go
package blobs
type Store interface {
    Stage(ctx, io.Reader, Expect) (Digest, Pin, error)
    OpenAuthorized(ctx, RecordGrant, Digest) (io.ReadSeekCloser, error)
    Transfer(ctx, Digest, Source) error
    Protect(ctx, ReferenceSet) error
    Progress() BlobProgress
}
```

```go
package trust
type Authority interface {
    Pair(ctx, PairingRequest) (PairingResult, error)
    Authenticate(ctx, Credential) (Principal, error)
    Authorize(Principal, Action) error
    Visibility(Principal) Visibility
    Revoke(ctx, PrincipalID) error
}
```

```go
package frontdoor
func Handler(Deps, []contracts.OperationDescriptor) http.Handler
```

```go
package runtime
func Start(ctx, Config) error
```

`Runtime` verbs are lifecycle behaviour invoked as `bridge <verb>`: `start`, `pair`, `join`, `token`, `doctor`, `restore`, `upgrade`, `stop` (MODULES-RESOLUTION C8: not a product CLI; only `bridge-lab` is a control tool).

---

## 6. Contracts

### 6.1 Bridge record

```json
{"collection":"reminders.reminder","id":"rmd_01J…","version":7,"source":"reminders",
 "observed_at":"2026-10-17T10:00:00.123Z","deleted":false,
 "coverage":{"fields":"complete","missing":[]},
 "data":{…}}
```

- `id`: bridge ID, assigned by State, prefixed by collection kind, ULID body. Stable across native identity changes via aliases (F11).
- `version`: per record, increments on each committed change within an epoch. Never compared across epochs.
- `source`: one of `messages|calendar|reminders|contacts|notes|usage|system`. It is the record's **visibility key**.
- References to other records use bridge IDs (`chat_id`, `list_id`, …).
- Unknown optional fields are preserved; unknown required semantics reject without advancing checkpoints (F15).

### 6.2 Collections (v1)

| Collection | Key data fields |
|---|---|
| `messages.chat` | `service` (iMessage/SMS/RCS), `style` (direct/group), `display_name`, `participants[]` (handle ids), `is_archived`, `last_read_at`, `capabilities` (per-chat allowed actions) |
| `messages.handle` | `address` (phone/email), `service`, `contact_id?` |
| `messages.message` | `chat_id`, `sender_handle_id?`, `is_from_me`, `sent_at`, `parts[]` (`{index, kind: text|attachment, text?, attachment_id?}`), `text` (joined plain text), `decode_status` (`decoded|partial|undecoded`), `raw_body_digest?`, `reply_to_id?`, `thread_originator_id?`, `edited_at?`, `edit_history[]`, `retracted_parts[]`, `retracted_at?`, `delivered_at?`, `read_at?`, `error_code`, `effect?`, `kind` (`message|group_event|system`), `group_event?` |
| `messages.reaction` | `message_id`, `part_index`, `sender_handle_id?`, `is_from_me`, `kind` (love/like/dislike/laugh/emphasize/question/emoji/sticker), `emoji?`, `removed` |
| `messages.attachment` | `message_id`, `filename`, `mime_type`, `uti`, `size`, `blob` (`{digest?, state: present|pending|offloaded|missing|over_cap|failed|disk_full}`) |
| `calendar.calendar` | `title`, `account`, `color`, `writable`, `type` |
| `calendar.event` | `calendar_id`, `series_id?`, `occurrence_date?` (original), `title`, `notes`, `location`, `url`, `start`, `end`, `all_day`, `time_zone`, `recurrence?` (rule on series), `is_exception`, `attendees[]` (read-only), `alarms[]`, `availability`, `status` |
| `reminders.list` | `title`, `account`, `color`, `writable` |
| `reminders.reminder` | `list_id`, `title`, `notes`, `url`, `due`, `due_all_day`, `start`, `priority`, `completed`, `completed_at`, `recurrence?`, `alarms[]` |
| `contacts.contact` | names (given/middle/family/nickname/prefix/suffix/phonetic), `organization`, `job_title`, `department`, `phones[]`, `emails[]`, `postal_addresses[]`, `urls[]`, `birthday?`, `dates[]`, `social_profiles[]`, `instant_messages[]`, `relations[]`, `image` (blob ref), `container_id`, `writable`, `note` (`unavailable` without entitlement) |
| `contacts.group` | `name`, `member_ids[]` (read-only in v1) |
| `notes.folder` | `title`, `account`, `parent_id?` |
| `notes.note` | `folder_id`, `title`, `plain_text`, `html?`, `created_at`, `modified_at`, `locked`, `attachments[]`, `content_status` (`complete|locked|offloaded|undecoded`), `editable` (`plain|none`) |
| `usage.device` | `name`, `platform` (mac/iphone/ipad), `build`, `is_local` |
| `usage.app_day` | `device_id`, `day` (local date + tz), `bundle_id`, `seconds_estimated`, `sessions`, `clipped_seconds`, `provenance` (`biome.app_infocus`), `decoder_version`, `latest_event_at`, `uncertainty` |
| `system.source` | per source: `state` (`ready|degraded|unavailable|permission_missing|unsupported`), `coverage`, `freshness` (`last_capture_at`, `capture_lag_ms`, `last_complete_scan_at`), `capabilities[]`, `reason?` |
| `system.command` | public status of commands visible to the principal (§11) |

`system.replica` status is local to each machine (not journaled): connection state, cursor, lag, blob backlog, last heartbeat.

### 6.3 Operation descriptors (handwritten, D2)

```go
type OperationDescriptor struct {
    Name        string
    Version     int
    Kind        OpKind
    Source      Source
    Scope       Grant
    Capability  string
    Input       any
    Output      any
    Description string
    Examples    []Example
}
```

`Kind` is `read` (served locally on any machine) or `command` (routed to CommandTarget). The table in `internal/contracts/operations.go` is the single list; Frontdoor generates REST routes, MCP tools and OpenAPI from it. Input/Output are Go types; JSON Schema for MCP/OpenAPI is derived by reflection plus explicit enums/descriptions in struct tags.

**Read operations** (all sources): `records.get`, `records.query` (collection + typed filters + order + cursor pagination), `search` (FTS over text fields within scope), `messages.thread` (chat history page with reactions/attachments inlined), `calendar.agenda` (time range), `reminders.open` (incomplete by list/due), `contacts.lookup` (by name/handle), `usage.summary` (device/day/app ranges), `sources.status`, `commands.status`, `commands.list`, `replica.status`, `blobs.get` (REST only streams bytes; MCP returns metadata and a local file path on the same machine).

**Command operations** (each gated by a capability; §9 lists per-source evidence):

| Source | Operations |
|---|---|
| messages | `messages.send` (existing chat or handle list; text and/or files), `messages.reply` (threaded reply to a message part), `messages.react` (add/remove tapback or emoji), `messages.edit`, `messages.unsend`, `messages.mark_read`, `messages.mark_unread`, `messages.delete_message`, `messages.delete_chat`, `messages.create_group`, `messages.rename_group`, `messages.add_member`, `messages.remove_member`, `messages.leave_group` |
| calendar | `calendar.create_event`, `calendar.update_event` (span `this|future`), `calendar.delete_event` (span `this|future`), `calendar.create_calendar` |
| reminders | `reminders.create`, `reminders.update`, `reminders.complete`, `reminders.uncomplete`, `reminders.delete`, `reminders.create_list` |
| contacts | `contacts.create`, `contacts.update`, `contacts.delete` |
| notes | `notes.create`, `notes.update` (plain text replace/append; rich edits reject), `notes.delete`, `notes.create_folder` |

Updates and deletes require `expected_version`. Unsupported combinations (e.g. `span=all`, editing past Apple's 15-minute/5-edit window, unsend after 2 minutes, membership change on an SMS group) reject before dispatch with `ErrUnsupported` and a reason.

### 6.4 Wire codecs and versions

- ApplePeer IPC: version `ipc/1`. Feed: `feed/1`. REST: `/v1`. MCP tools carry the operation version in their description. Collection schema versions per collection (`schema: 1`). Each evolves separately; negotiation at connection open picks the highest common version or refuses.
- JSON everywhere (UTF-8, RFC 3339 UTC timestamps with milliseconds, durations in ms). Frames: 4-byte big-endian length + JSON object with `"t"` (type) field. Max frame 256 KiB metadata (larger snapshot pages split), blob chunks 1 MiB as binary WebSocket frames.
- Golden fixtures in `internal/contracts/testdata/` are decoded by both Go and Swift checks so the two sides cannot drift.

---

## 7. The Apple seam (ApplePeer IPC)

Design B (duplex conversation) from MODULES-astra §4: Swift owns capture progress; Go learns durable facts and execution evidence.

### 7.1 Transport and authentication

- Owner-only Unix socket at `~/Library/Application Support/AppleBridge/run/peer.sock` (dir 0700). The gateway listens; the Helper connects.
- Authentication: peer UID check (`getpeereid`) plus an installation secret (32 random bytes, file 0600 in the app's support dir, Keychain item in packaged builds) proven by HMAC over a nonce exchange. Both sides prove possession.
- `ownerFence`: monotonically increasing integer issued by the gateway per Helper session; every Dispatch carries it; a stale fence is rejected by the Helper (prevents a zombie helper from acting).

### 7.2 Messages

```text
G→H Open(protocol, ownerFence, checkpoints{source→checkpoint}, byteCredit)
H→G Ready(protocol, capabilities{source→[capability]}, coverage{source→Coverage}, helperBuild, osBuild)
H→G Capture(source, generation, predecessor, checkpoint, observations[], scanID?, scope?, completionEvidence?)
G→H Committed(source, checkpoint, credit)
H→G SourceState(source, state, coverage, capabilities, reason)          (permission/capability changes)
G→H Prepare(commandID, principal, requestHash, action, expectedState, expiresAt)
H→G Prepared(commandID, ticket, nativeState, route, fingerprint, candidateIDs) | Rejected(commandID, error)
G→H Dispatch(ticket, ownerFence)
H→G Outcome(commandID, disposition, evidence)
G→H Resolve(commandID)
H→G Outcome(commandID, disposition, evidence)
G→H BlobRead(handle, offset, length)
H→G BlobChunk(handle, offset, bytes, digest, eof) | BlobUnavailable(handle, reason)
both Ping/Pong; Close(reason)
```

### 7.3 Capture rules

- An **observation** is `{kind: upsert|absent|alias, collection, native_key, scoped_alias?, data, observed_at}`. Native references stay behind IPC except opaque scoped aliases; State assigns bridge IDs.
- `Capture` is atomic in Go: records + aliases + journal + checkpoint commit in one transaction; then `Committed`. The Helper does not advance its in-memory checkpoint until `Committed`.
- `generation` identifies one consistent native identity space (e.g. chat.db file identity + schema hash; EventKit store; Contacts history token lineage). A new generation resets capture identity, not gateway history. `predecessor` must equal the last committed checkpoint for that source, or Go answers `Reset(source)` and the Helper rescans.
- A Capture may carry a private `blob_handles` map (native blob handle → collection/record key). Handles are opaque Helper references used only for `BlobRead`; they are never journaled or replicated.
- `absent` observations are only valid inside a scan with `scanID`, `scope` and `completionEvidence: complete`. Go ignores absence without completion evidence and records a `degraded` coverage note. Candidates for deletion are revalidated, and records updated after the scan started are protected (F2, F13).
- Live observations outrank backfill: the Helper keeps two queues (live, bulk) and always drains live first. Backfill revalidates before emitting so older data never overwrites newer observations (F14).
- `byteCredit` bounds in-flight capture bytes (4 MiB). The Helper stops emitting when credit is exhausted.
- A failed capture never implies an empty source (C4/C9).

### 7.4 Commands across the seam

- Prepare reserves an execution slot and returns a single-use, expiring (≤ 30 s), principal/hash-bound, session-fenced ticket. Prepare also captures pre-dispatch evidence: native state, route, payload fingerprint, candidate IDs (e.g. current max message ROWID in the chat).
- Go persists Prepared evidence (`prepared` transition) **before** sending Dispatch.
- The Helper's **dispatch-start guard**: before acting, it durably writes `{commandID, ticket, startedAt}` to `dispatch-guard.log` (append-only, fsync) in its support dir. On Helper start, any started-without-outcome entry is reported as `Outcome(indeterminate)` (C1).
- Dispatch rejects busy/expired/used tickets (`Rejected`) without queuing.
- After an effect, the Helper observes native state for evidence (bounded wait per operation: messages 10 s, EventKit/Contacts 3 s, Notes 5 s) and returns `Outcome` with a disposition.
- `Resolve` re-examines native state for an indeterminate command using stored evidence; it can only strengthen (`indeterminate → observed|absent_confirmed`), never trigger a new effect.

### 7.5 Dispositions

`rejected_before_dispatch`, `failed_no_effect` (native error with proof nothing happened), `locally_observed` (effect visible in Mac state), `delivered` (messages: `is_delivered`), `delivery_failed` (messages: error code after send), `indeterminate`, `expired` (never dispatched). Zero native error never proves delivery (F5).

---

## 8. The Source seam (Swift)

```swift
protocol Source: Sendable {
    var name: SourceName { get }
    func open() async -> SourceReadiness
    func capture(_ resume: Checkpoint?, into sink: ObservationSink) async throws
    func prepare(_ command: NativeCommand) async throws -> Prepared
    func dispatch(_ prepared: Prepared) async -> Evidence
    func resolve(_ command: CommandID, evidence: Evidence) async -> Evidence
}
```

- `capture` runs for the life of the session: initial/resumed scan, then live following, periodic complete reconciliation. It emits through the sink, which applies credit and live/bulk priority.
- Typed action unions per source (`MessagesAction`, `CalendarAction`, …) keep domain distinctions. Read-only sources reject `prepare` with `unsupported`.
- One **UI actor** (`@globalActor UIActor`) serializes all Accessibility and AppleScript actions across sources. It verifies the target (window, conversation, message bubble) before acting, detects human interference (frontmost app or focused element changed unexpectedly, user input events within the action) and aborts with `indeterminate` if interference happens mid-action, `rejected_before_dispatch` if before.
- `AgendaGate` is shared by Calendar and Reminders: one `EKEventStore`, authorization, `EKEventStoreChanged` debounce (50 ms), refetch, account writability. Each source keeps its own checkpoints and scan-completion evidence (MODULES-RESOLUTION D1).

---

## 9. Sources at full depth

Every source publishes `system.source` with state, coverage, freshness and capabilities, and exposes capabilities individually gated on OS build and proven behaviour. A capability is advertised only after its real-level scenario passed on this OS build (recorded in `features.yaml`); otherwise operations return `ErrUnsupported` with reason `capability_unproven`.

### 9.1 Messages

**Read path.** `~/Library/Messages/chat.db` opened `SQLITE_OPEN_READONLY` with WAL (needs FDA). Schema probed at open (column list hashed into the generation); missing columns become explicit coverage gaps, never crashes. Watch `chat.db`, `chat.db-wal`, `chat.db-shm` and the directory with a `DispatchSource`/FSEvents watcher, re-armed after file replacement (inode change), debounce 25 ms, plus a 2 s polling fallback when no events arrive while the WAL size changes, plus a complete reconciliation every 15 min and after wake.

**Change algorithm (F8).**
1. Inserts: `message.ROWID > lastRowID` (scan position, database-local).
2. Mutations: rows whose mutable columns changed: `date_delivered, date_read, is_delivered, is_read, date_edited, date_retracted, message_summary_info, error, is_finished, item_type, attributedBody length` — detected by a bounded rescan of the most recent N=2,000 messages plus any message referenced by new rows (`associated_message_guid`, `reply_to_guid`, `thread_originator_guid`) on every change event, plus the periodic complete reconciliation which hashes every row.
3. Chats, handles, chat membership (`chat_handle_join`), attachments (`message_attachment_join`, `attachment.transfer_state`): full-table hash diff on change (tables are small) and on reconciliation.
4. Deletions: a message/chat missing from a **complete** reconciliation scan of a chat is published absent; `deleted_messages`/`sync_deleted_*` tables are used as corroborating evidence.
5. Database replacement (inode or schema hash change) starts a new generation with a full rescan.

**Identity.** GUIDs (`message.guid`, `chat.guid`, `attachment.guid`) are native keys; ROWIDs are scan positions only.

**Decoding (F9).** `attributedBody` typedstream is decoded by `Decoders.TypedStream` (ported from the format documented by imessage-exporter, written fresh; no GPL code copied) into parts: text runs, attachment placeholders (`__kIMFileTransferGUIDAttributeName`), mentions, part indices. `message_summary_info` (binary plist) yields edit history and retracted parts. Undecodable payloads keep `decode_status=undecoded` and `raw_body_digest`; they never become empty text. Reactions: rows with `associated_message_type` 2000–2007 (add) / 3000–3007 (remove) plus `associated_message_emoji`, target `p:<part>/<guid>`.

**Attachments.** `attachment.filename` resolved under `~/Library/Messages/Attachments`; transfer state mapped to `present|pending|offloaded|missing`. Blob reads go through `BlobRead`. Messages never forces download from iCloud in v1.

**Actions and mechanisms.**

| Operation | Mechanism | Evidence | Notes |
|---|---|---|---|
| `send` to existing chat (text/files) | AppleScript `send … to chat id "<raw chat.db guid>"` — proven on 26.7.1 with the raw `any;-;` GUID; the normalized `iMessage;-;` form fails with -1728 | new `is_from_me` row in that chat after `candidateIDs.maxRowID` with matching fingerprint (text hash / attachment names); `error`, `is_sent`, `is_delivered` | ghost-send detection: row with empty text and `service=SMS` unjoined → `delivery_failed` evidence (F10) |
| `send` to new direct handle | AppleScript `send … to participant/buddy` on the explicit service | same | service routing explicit: iMessage, or SMS/RCS only when iPhone relay is available (capability probe) |
| `send` files | file staged into `~/Library/Messages/Attachments/AppleBridgeOutgoing/<cmd>/` (Messages-readable), sent per file | each part tracked separately (text vs each file) | partial sends reported per sub-effect |
| `reply` | Accessibility: open chat, locate bubble by GUID-matched text/time, context menu "Reply", type, send | row with `thread_originator_guid` = target | UI actor |
| `react` / remove | Accessibility: bubble → Tapback menu | row with `associated_message_guid` = target, type 2000+k / 3000+k | |
| `edit` | Accessibility: bubble → Edit, replace text, confirm | `date_edited` changes and `message_summary_info` contains new text | Apple limits: 15 min, 5 edits, iMessage only |
| `unsend` | Accessibility: bubble → Undo Send | `date_retracted` set / part retracted | 2 min, iMessage only |
| `mark_read` / `mark_unread` | Accessibility (context menu on conversation) | `is_read` / chat `last_read_message_timestamp` | |
| `delete_message` / `delete_chat` | Accessibility (context menu → Delete, confirm) | row/chat absent in complete scan | test-created data only in tests |
| `create_group` | Accessibility (new message, multiple recipients, send first message) | new chat with style group and participants | capability proven separately (F10) |
| `rename_group`, `add_member`, `remove_member`, `leave_group` | Accessibility (conversation details) | `group_action_type` / `group_title` rows, `chat_handle_join` changes | iMessage groups only; RCS rename if proven |

Every UI action records a before/after screenshot path in evidence (stored in the Helper support dir, excluded from git) where disk state alone cannot prove the visible effect.

**Riskiest unknowns (prove first in M1):** AppleScript send on macOS 26.7.1 — **proven 2026-10-17**: send to self via raw `any;-;` chat id produced `is_sent=1, is_delivered=1`; the outgoing row has `text` NULL (content only in `attributedBody`), and a self-chat send also produces an incoming echo row, so correlation must decode `attributedBody` and distinguish the echo. Still open: Accessibility tree of Messages 26 for bubble targeting with SIP on.

### 9.2 Calendar

- EventKit full access (`requestFullAccessToEvents`). Window: `[today-730d, today+730d]` in local time, queried in chunks of ≤ 1 year (EventKit truncates > 4 years; F12). Slides daily at local midnight: records leaving the window are published as evictions, not deletion: the journal change carries `op: evict` and the wire field `tombstone_reason: "window_evicted"` (optional; absent means an ordinary deletion). Replicas remove evicted records from the collection but must not report them as deleted to callers.
- Identity (F11): native key = `calendarItemIdentifier` + `occurrenceDate` (original) for occurrences; aliases also store `calendarItemExternalIdentifier` scoped by calendar/account. On local-identifier churn after full sync, the external identifier + occurrence date re-links to the same bridge ID.
- Series: each occurrence in the window is a `calendar.event`; the series rule lives on every occurrence (`recurrence`) with `series_id`; detached occurrences have `is_exception=true`.
- Change detection: `EKEventStoreChanged` → debounce → refetch window in chunks, nearest chunk (±30 days) first for latency, hash diff against the last committed set → observations. Complete scan of the window produces `absent` with completion evidence. Periodic full reconciliation every 15 min.
- Commands: create (calendar must be writable), update with span `this|future` (`EKSpan`), delete with span. `all` rejects. Expected version checked against a fresh native refetch (F7). Per-series serialization.
- All-day events keep floating dates; time zones preserved; DST tested.

### 9.3 Reminders

- EventKit full access to reminders. All lists, all reminders (completed included) fetched; reminders have no window.
- Recurring reminders: only the next incomplete instance is obtainable (F12); coverage notes it.
- Commands: create/update/complete/uncomplete/delete/create_list. Evidence: refetch by identifier shows the new field values / absence.
- Identity: `calendarItemIdentifier` with external-identifier alias, scoped by list.

### 9.4 Contacts

- `CNContactStore` full fetch with all keys allowed without entitlement (`CNContactNoteKey` excluded → `note: unavailable`). Unified contacts. Groups and membership read.
- Change tracking: `CNChangeHistoryFetchRequest` with the history token stored as the source checkpoint; changes and the new token commit together (F13). `CNChangeHistoryDropEverythingEvent` → full rescan in a new generation (reconciliation, never deletion). `CNContactStoreDidChange` notification triggers a fetch (debounce 50 ms).
- Commands via `CNSaveRequest`: create (into the default writable container), update (field patch, expected version), delete. Respect container writability. Group management later (not v1).
- Contact images are blobs.

### 9.5 Notes

- Read: consistent copy of `NoteStore.sqlite` (+WAL) via SQLite backup API from a read-only connection, every change event (FSEvents on the group container, debounce 100 ms) and every 5 min. Decode `ZICNOTEDATA.ZDATA` (gzip + protobuf) with `Decoders.NotesProto` to plain text and a minimal HTML; tables/checklists/attachments listed. Locked notes: `content_status=locked`, title only. Schema probed; unknown → `undecoded` with raw digest.
- Write: AppleScript (`make new note at folder … with properties {body: html}`, `set body of note id …`, `delete note id …`, `make new folder`). Only plain-text create/replace/append are supported; notes containing tables, checklists, drawings or attachments reject updates with `ErrUnsupported(reason: rich_content)` before dispatch (D6). Evidence: re-read note via AppleScript id + next capture.
- Identity: Core Data `ZIDENTIFIER` UUID; AppleScript id `x-coredata://…/ICNote/p<pk>` stored as alias.

### 9.6 Usage (Screen Time, read-only)

- Read `~/Library/Biome/streams/restricted/App.InFocus/local` and `…/remote/<device-uuid>/` SEGB files (FDA). Device names/platforms from `~/Library/Biome/sync/sync.db` `DevicePeer`.
- Decoder (`Decoders.SEGB` + `InFocus`): per record field 3 = starting flag, field 4 = Mac-epoch double, field 6 = bundle id (see `probe/infocus_usage.py`). Pair start/stop into sessions; cap a session at 4 h; a new start closes the previous session.
- **Lock/sleep clipping (Mac devices):** clip sessions with lock/display evidence where it exists: the local Mac uses `log`-independent sources in this order: Biome `Device.Display`/`ScreenLock`-family streams if present on this build; otherwise `loginwindow`/`com.apple.ScreenSaver.Engine` focus events (lock screen = focus moves to `loginwindow` or screen saver), and sleep/wake from `pmset -g log`-equivalent IOKit power events recorded by the Helper going forward. Clipped seconds are published separately (`clipped_seconds`). The exact streams available on 25G241 are probed in M4 before coverage is advertised.
- Output: `usage.device`, `usage.app_day` (estimates, `uncertainty` text, `decoder_version`, `latest_event_at`). Remote devices arrive in batches (~20 h lag observed); freshness reports the newest event per device.
- knowledgeC `/app/usage` is corroboration only (never summed). ScreenTimeAgent stores are vaulted (EPERM with SIP on) → coverage says `settings_parity: unproven`.

---

## 10. State

One deep module (C2) over one SQLite database per machine (`state.db`, WAL, `synchronous=FULL` for journal commits, file mode 0600).

### 10.1 Tables (internal; not a contract)

- `meta(key, value)`: `epoch` (ULID), `admission_era`, `role`, `schema_version`, `gateway_id`.
- `records(collection, id, version, source, deleted, data, observed_at, seq)` + FTS5 `records_fts(text)` over designated text fields per collection.
- `aliases(source, generation, native_key, scoped_alias, record_id)`.
- `journal(seq INTEGER PRIMARY KEY, epoch, collection, id, version, source, op (upsert|delete|evict), data, committed_at)`.
- `checkpoints(source, generation, checkpoint, updated_at)`.
- **Command ledger in a separate database file `ledger.db`** (same directory, same protections): `commands(id, principal, request_hash, op, args, state, admission_era, created_at, expires_at, evidence, disposition, commit_seq, history)` and `dedupe(command_id, principal, request_hash, admission_era)`. It is retained independently of journal retention and mirror rebuilds, and **restore never replaces it** (C1, F4). Journal rows and records referencing commands carry the command ID only.
- Replica-only: `cursor(epoch, subscription_revision, sequence)`, `staging_*` tables per snapshot generation.

### 10.2 Behaviour

- `CommitCapture`: one transaction: resolve aliases → bridge IDs, upsert/delete/evict records with version+1 where data changed (no-op if identical hash), append journal rows, write checkpoint. Returns the commit seq range.
- `TransitionCommand`: compare-and-set on command state (`admitted → prepared → dispatched → {observed|delivered|delivery_failed|failed|indeterminate} …`); invalid transitions error. Status changes append a `system.command` journal row (scoped to principal and source).
- `FreezeSnapshot(S)`: a read transaction pinned at seq S (SQLite snapshot isolation in WAL), producing pages of visible records plus count and digest (SHA-256 over sorted `(collection,id,version)`); pins journal retention ≥ S until the lease expires (default 10 min, extended while pages flow).
- `InstallSnapshot` (replica): stage pages into a new staging generation; validate count+digest; one transaction swaps generation and sets cursor = (epoch, rev, S), removing records absent from the snapshot (F2).
- `ApplyBatch` (replica): reject if epoch/revision differ; ignore if `through ≤ cursor`; require `after == cursor.sequence`; apply changes and cursor in one transaction; return new cursor. Partial overlap or gap → error, never advance (F3).
- Journal retention: keep ≥ 7 days and ≥ 1,000,000 rows (configurable caps); never truncate past an active snapshot lease; tombstones retire only past the retention floor. Disk-full: stop capture (source `degraded`, reason `capacity`), never corrupt; resume when space returns.
- Restore (Runtime): replace `state.db` from the backup, keep the live `ledger.db`, rotate `epoch` and `admission_era` before serving.

**Command IDs.** Command IDs are opaque caller-selected strings (1–128 chars, `[A-Za-z0-9._:-]`), independent of replication epochs and cursors; callers never need to know internal state to submit. The gateway binds each ID to its authenticated principal, canonical request hash and the server-assigned admission era at admission. An ID already in the ledger is never executed again: same principal+hash returns the stored status; a different hash is `conflict(id_reused)`; an ID admitted in an older era is lookup-only. Because the ledger survives restore, retries after a restore are recognized. If `ledger.db` itself is lost (machine loss), previously executed IDs cannot be recognized; this is a documented limit (§22), and Runtime refuses to start the gateway role when `ledger.db` is missing but `state.db` shows prior commands, until the operator runs `bridge restore --accept-ledger-loss`, which starts a new admission era.

---

## 11. Operations and commands

### 11.1 Reads

`Read(Query)` on any machine reads local State through Visibility; never calls Apple. Results include `cursor` (local head) so callers can relate reads to waits.

### 11.2 Command lifecycle (gateway)

```text
Submit(principal, {id, op, args, expected_version?, expires_at?})
  1 validate shape (Contracts) and authorize (Trust) — reject before any effect
  2 dedupe: same id+principal+hash → return existing status; same id different hash → ErrConflict(id_reused);
    id from an older admission era → lookup only (no execution)
  3 admit: ledger row (admitted), per-object/series serialization slot
  4 conflict check: expected_version vs current State; mismatch → ErrConflict with current record
  5 Prepare over ApplePeer → persist prepared evidence (route, fingerprint, candidates)
  6 recheck authorization + server-UTC expiry (monotonic timer in-process) immediately before dispatch
  7 Dispatch → dispatched
  8 Outcome → final disposition; evidence persisted; commit_seq of the observing capture recorded
```

- Expiry: default and maximum TTL per operation (messages 60 s/5 min, others 5 min/1 h). Clock skew: a request whose `expires_at` is in the past or beyond max TTL relative to the gateway clock rejects; requests carry no client clock assumptions otherwise. Wake/restart: admitted-not-dispatched commands are re-evaluated; if expired → `expired`.
- Helper unavailable → `ErrUnavailable` immediately (fail fast).
- Lost response: the caller retries **with the original ID**, getting the stored status. Never automatic redispatch of `dispatched`/`indeterminate`.
- Correlation is conservative: evidence must match fingerprint **and** be after candidate IDs **and** be unique; two identical candidate rows (e.g. a manual identical send) → `indeterminate` with both candidates listed.
- Stronger later evidence (from `Resolve` or capture) may move `indeterminate → locally_observed/delivered/failed_no_effect`, appending to `history`.
- Status returns `commit_seq`; `Wait(cursor)` on a replica blocks until local cursor ≥ that sequence, so callers can read their own write locally.

### 11.3 Replica

`CommandTarget = RemoteGateway`: HTTPS POST to the gateway peer listener over mTLS with a **delegated identity** header: `{replica_id, local_principal, local_grants, command_id, request_hash, iat}` signed with the replica's device key. The gateway verifies the replica certificate, signature freshness (≤ 60 s), that `local_grants ⊆ replica's delegable grants`, and authorizes against them. Disconnected → `ErrUnavailable` immediately. Response lost → status lookup by the same ID.

---

## 12. Replication (feed protocol `feed/1`)

WebSocket over mTLS at `wss://<gateway>:7443/v1/feed`. Messages are JSON text frames (metadata) and binary frames (blob chunks). The ApplePeer IPC uses 4-byte big-endian length-prefixed JSON frames instead (§6.4); the feed relies on WebSocket framing. Blob chunk binary frame: 64-byte header = bytes 0–31 SHA-256 digest of the whole blob, 32–39 big-endian uint64 offset, 40–43 big-endian uint32 payload length, 44–63 zero; then the payload (≤ 1 MiB).

```text
R→G Hello(replicaID, cursor?{epoch, subscriptionRevision, sequence}, schemaOffer, credits)
G→R Welcome(gatewayID, epoch, subscriptionRevision, scope, head, retentionFloor, schema)
G→R SnapshotBegin(id, S, scope, count, digest, expiresAt)
G→R SnapshotPage(id, ordinal, records[])
G→R SnapshotEnd(id)
G→R Batch(epoch, subscriptionRevision, after, through, changes[])
R→G Ack(cursor)                          credits replenish on Ack
G→R Reset(reason, retentionFloor)        replica must resnapshot
G→R Heartbeat(head, at) every 5 s;  R→G Heartbeat
R→G BlobWant(digest, offset)  G→R BlobChunk(binary)  G→R BlobDone(digest) | BlobUnavailable(digest, reason)
```

Rules (F2, F3, D4):
- `(after, through]` covers every sequence, including changes filtered out by scope; empty batches advance watermarks.
- Resume requires same epoch and subscription revision and `retentionFloor ≤ cursor.sequence ≤ head`; otherwise snapshot.
- Snapshot at S, then retained S+1.. replay, then live — from the same journal iterator so there is no gap at the catch-up→live transition.
- Interrupted snapshot staging is discarded unless resumed within its lease.
- Credits: 4 MiB outstanding metadata; replica acks after each committed batch. A consumer that doesn't ack for 30 s or exceeds the server's 16 MiB send buffer is disconnected; on reconnect it resumes or resnapshots.
- Replicas never poll for data. They detect disconnection locally (no heartbeat for 15 s → `system.replica.state=disconnected`, reads continue with staleness reported).
- Scope change for a replica → new subscription revision → purge + snapshot of the new scope.
- Local SSE (`GET /v1/changes?after=`) reuses the same framing semantics for local subscribers, filtered by the caller's Visibility.

Budgets (provisional, benchmarked in M0/M5 on the Air): 256 KiB frames, 1 MiB blob chunks, 4 MiB credit.

---

## 13. Blobs

- Content-addressed immutable files `blobs/sha256/ab/cdef…` (0600), staging under `blobs/staging/`.
- `Stage` writes and digests; returns a pin. Pins precede record commits and last until `Protect(referenceSet)` makes protection durable (D5).
- `OpenAuthorized(recordGrant, digest)`: the grant comes from an authorized State read of a record that references the digest. Digest knowledge alone authorizes nothing.
- Gateway: blobs fetched from the Helper by handle on first need (eager for new attachments up to the gateway cap; the gateway's cap defaults to unlimited for Messages attachments present on the Mac).
- Replica: `Transfer` downloads every referenced blob by default (chunked, resumable from offset, digest verified) up to the machine cap (default 20 GiB; configurable). States: `present`, `pending`, `offloaded` (not on Mac), `missing`, `over_cap`, `failed`, `disk_full`.
- Live metadata preempts bulk chunks (separate priority lanes in the feed writer).
- Garbage collection: blobs unreferenced by records, snapshots, or commands, and unpinned, are deleted after 24 h.

---

## 14. Trust

- **Principals.** `replica:<id>` (paired machine; trusted replica identity with a subscription scope and delegable grants) and `token:<id>` (API caller on one machine, scoped). Local file owners bypass API scopes; revocation cannot erase copied data (documented, F16).
- **Grants.** `<source>:read`, `<source>:write` for `messages|calendar|reminders|contacts|notes|usage`, plus `system:read`, `admin`. `<source>:write` implies `<source>:read`. A command's status and result are always visible to the principal that issued it through `commands.status`.
- **Visibility.** Computed once per principal: set of readable sources. Applied in State reads, snapshot freezing, journal iteration (filtered changes still advance watermarks), FTS search, blob grants, command lists/results, SSE and MCP.
- **Pairing.** On the gateway: `bridge pair --name ubuntu --scope messages:write,…` prints a one-time code (`ab1-<base32>`; 160-bit secret, expires in 10 min, 5 attempts, single use) containing gateway address + gateway certificate fingerprint. On the replica: `bridge join <code>` generates an Ed25519 key and self-signed client certificate, connects over TLS pinning the fingerprint, proves the code with HMAC over the client cert, receives its replica ID, scope and the gateway cert. Replayed or expired codes fail.
- **Tokens.** `bridge token create --scope reminders:write` on any machine creates a local API token (shown once; stored hashed). Tokens are per machine; a replica's tokens cannot exceed its delegable grants.
- **Credential stores.** macOS: Keychain (`security` framework via cgo-free `security` CLI wrapper in Go, Keychain APIs in Swift). Linux: `LinuxSecretStore` = libsecret via `secret-tool` when available, else a 0600 file in `$XDG_CONFIG_HOME/apple-bridge/secrets/`. Both adapters pass the same credential lifecycle scenario.
- **Rotation/Revocation.** `bridge revoke <principal>` closes live connections immediately and rejects further auth; `bridge rotate` re-issues gateway certs (replicas re-pin through a signed rotation message).

---

## 15. Frontdoor

- Listens on `127.0.0.1:7780` (configurable; never `0.0.0.0` by default). Every request authenticated (`Authorization: Bearer <token>`), even on localhost. `Host` must be `127.0.0.1:<port>`/`localhost:<port>`; any `Origin` header not on an explicit allow list rejects (browser-origin/DNS-rebinding defense). Bounded body (1 MiB, files via multipart up to 100 MiB to staged blobs). Rate limit per principal. Audit log of commands with redacted arguments (no message text; lengths and hashes only).
- **REST**: each descriptor → `POST /v1/op/<name>` (JSON in, JSON out). Convenience GETs: `GET /v1/records/<collection>/<id>`, `GET /v1/blobs/<digest>?record=<id>`, `GET /v1/changes` (SSE), `GET /v1/openapi.json`, `GET /v1/health` (unauthenticated liveness only, no data).
- **MCP**: Streamable HTTP at `/mcp` (same bearer auth), using the official Go SDK. Each descriptor → one tool (`messages_send`, `records_query`, …) with JSON Schema from the descriptor, the handwritten description and examples. Command tools accept an optional `command_id` (generated if absent and returned) and `wait_ms` (wait for a final disposition up to 15 s).
- The same behaviour scenarios run through both REST and MCP (D2).
- Error format (both): `{"error":{"code":"conflict","message":"…","current":{…}?,"retryable":false}}`.

---

## 16. Runtime

- `bridge start --role gateway|replica --home <dir>`: singleton lock (`flock` on `<home>/lock`), migrations, backup-before-migrate, wiring, readiness. Gateway also supervises the Helper: in dev the Helper is launched via `open -a` (so TCC attributes it to the app bundle); in packaged builds by the app/SMAppService.
- Readiness (`GET /v1/health` + `sources.status`) reports per source ready/degraded/stale, not just liveness.
- `doctor`: permissions, disk encryption (FileVault on macOS, LUKS check on Linux → warning), file modes, clock skew vs gateway, versions.
- `restore <backup>`: install backup read-only; rotate epoch and admission era; then serve.
- Sleep: gateway holds an `IOPMAssertion`-equivalent via the Helper (`ProcessInfo.beginActivity(.idleSystemSleepDisabled)`) while serving. Logout/sleep/shutdown/FileVault pre-login are outages; replicas detect them independently.
- **Packaging (M5):** `Apple Bridge.app` contains the Helper (main executable, UI for onboarding), `bridge` in `Contents/MacOS`, LaunchAgent plist registered with `SMAppService.agent`. Signed with Developer ID + hardened runtime + notarized (needs Johan's Developer ID; see §22). Foreground onboarding requests FDA (opens System Settings pane), Automation (Messages, System Events), Accessibility, Contacts, full Calendar, Reminders. Dev builds use a stable self-signed code-signing identity created on the Air so TCC grants survive rebuilds.
- Files: `~/Library/Application Support/AppleBridge/` (gateway) and `$XDG_DATA_HOME/apple-bridge/` (Linux replica); all 0600/0700 including WAL, indexes, blobs, ledger, logs.

---

## 17. Harness (`bridge-lab`)

`cmd/bridge-lab` + `lab/`. The harness is also the operator's control tool (JSON output on every verb).

```text
bridge-lab start  --level sim|real [--replica local|ubuntu] [--world <name>]
bridge-lab act    <world> <actor-action-json>
bridge-lab await  <world> <condition-json> --timeout
bridge-lab fault  <world> <fault>            kill-gateway|kill-helper|partition|heal|fill-disk|clock-jump|drop-response|…
bridge-lab assert <world> <assertion-json>
bridge-lab cleanup <world>
bridge-lab run    <scenario.yaml|dir>... --level sim|real|replica|agent [--repeat N] [--json]
bridge-lab features [--check] [--html out.html]
```

### 17.1 World

Owns processes, isolated homes (`/tmp/ablab/<world>/…` on each host), ports, network fault proxy (a TCP proxy between replica and gateway owned by the lab that can partition, delay, or cut mid-frame), and artifact cleanup. Hosts: `air` (local when the lab runs on the Air) and `ubuntu` (driven over `ssh ubuntu` with a control master). `SimEnvironment` = gateway + SimApplePeer + N replicas, all real binaries. `RealAirEnvironment` = gateway + the real Swift Helper on the Air (+ Ubuntu replica for the replica level).

### 17.2 SimApplePeer

A Go process (`bridge-lab simapple`) speaking `ipc/1` exactly like the Helper, holding an in-memory fake Apple for all six sources, with a control socket for the lab: `seed`, `external-change` (simulates a change made on the iPhone/another device), `inspect` (SimTruth), and faults: `crash-after-effect`, `crash-before-outcome`, `duplicate-capture`, `permission-loss`, `schema-change`, `db-replacement`, `drop-everything` (Contacts), `slow-capture`, `identical-manual-send`, `ui-interference`, `ambiguous-outcome`. It enforces the same native rules as Apple where they affect behaviour (edit window, unsend window, span semantics, writability, group service limits).

### 17.3 Truth

`Control.assert` uses Truth, never the product, to establish Apple facts.
- **SimTruth**: asks SimApplePeer `inspect`.
- **AppleTruth**: separate Swift package `lab/truth/native` (binary `apple-truth`) that shares **no code** with `native/`. Verbs: `observe <query>` (read-only: chat.db rows, EventKit items, CNContacts, Notes via AppleScript/SQLite copy, Biome files) and `fixture <action>` (writes confined to tagged test data: create/modify/delete in the "Bridge Test" calendar/list/folder, Bridge Test contacts; send a self-message via AppleScript to simulate an external change; cleanup of tagged data). `screenshot` captures the Messages window when only the UI can prove an effect. An unavailable check reports `unverified`, never pass.

### 17.4 Scenarios

YAML under `lab/scenarios/<source>/<ID>.yaml`; they compose existing harness actions (not a workflow language).

```yaml
id: REM-CREATE-EXTERNAL
feature: reminders.read
levels: [sim, real, replica]
given: [{world: {replicas: 1}}, {fixture: {reminders.list: "Bridge Test"}}]
when:  [{external: {reminders.create: {list: "Bridge Test", title: "[ab-test {{run}}] milk"}}}]
then:
  - {await: {on: replica0, record: {collection: reminders.reminder, where: {title: "[ab-test {{run}}] milk"}}, within_ms: 1000}}
  - {truth: {reminders.reminder: {title: "[ab-test {{run}}] milk", exists: true}}}
cleanup: [{fixture: {cleanup: reminders}}]
```

**Automatic assertions** on every sync scenario (the runner adds them; scenarios can't opt out):
1. **Convergence**: after quiescence, every replica's visible records equal the gateway's for its scope, and the gateway equals Truth within declared coverage (collections and fields listed per source in `lab/truth/coverage.yaml`).
2. **Latency**: named endpoints on the lab's monotonic clock: `effect_done` (fixture/act returns), `gateway_visible`, `replica_visible:<name>`, `blob_complete`; reports p50/p95/max and timeouts per endpoint pair; the `< 1 s` target applies to `effect_done → replica_visible` for metadata at the real level (sim also < 1 s).
3. **Transactional replica application**: replica cursor and records always consistent (checked by kill-during-apply runs and a post-run integrity check: each journal seq applied exactly once).
4. **No false deletion**: no record disappears from any replica unless Truth confirms absence (or window eviction).
5. **No scope leak**: a scoped token on every replica cannot see out-of-scope records through any path (REST query, search, SSE, MCP, blobs, commands).

Command scenarios additionally assert **no automatic redispatch** of ambiguous effects (Truth counts effects) and truthful indeterminate status.

### 17.5 AgentRunner

Runs on Ubuntu: `codex exec` with model `gpt-6-luna`, MCP configured to the Ubuntu replica's `/mcp` with a scoped token, no shell or file tools beyond what Codex requires (`--sandbox read-only`, MCP only instructed). Each natural-language task runs 3× and is graded by Truth and replica state, never by the agent's claims. Tasks live in `lab/agent/tasks/*.yaml`.

### 17.6 FeatureGraph

`features.yaml` (root) is the DAG feature → requirement → module → scenarios → levels → status → evidence. `tools/featuregraph` checks: every feature has ≥ 1 scenario; every scenario file referenced exists and references its feature; dependencies acyclic; `done` requires every required level's latest evidence passing (`lab/evidence/<level>/<scenario>.json`, redacted); renders `design/features.html`.

### 17.7 Evidence

`lab/evidence/<level>/<scenario>.json`: run id, git sha, timestamps, assertion results, latency stats, Truth checks — no personal data (text replaced with hashes, handles with roles `self`, `ben`). Private raw traces go to `lab/evidence/private/` (gitignored).

---

## 18. E2E scenario catalogue

Levels: **S** = sim, **R** = real (Air), **P** = replica on Ubuntu, **A** = MCP agent. Every scenario is driven through REST unless noted; scenarios marked "both" also run through MCP via ScriptedClient. All sync scenarios get the five automatic assertions. Agents add scenarios here when they find a gap.

### 18.1 Core (M0)

| ID | Scenario | Levels |
|---|---|---|
| CORE-BOOT | Gateway + SimApplePeer + replica start; Ready; sources report state; replica snapshots and converges | S P |
| CORE-LIVE-LATENCY | 200 external changes at 5/s; every change visible on replica; p95 < 1 s | S R P |
| CORE-SNAPSHOT-UNDER-LOAD | Replica bootstraps while 1,000 changes stream; converges with no gap at catch-up→live | S |
| CORE-SNAPSHOT-INTERRUPT | Kill replica mid-snapshot; restart; staging discarded or resumed; converges | S |
| CORE-RESUME | Partition replica 30 s during changes; heal; resumes from cursor without snapshot | S P |
| CORE-RETENTION-RACE | Replica offline beyond retention; on reconnect gets Reset → resnapshot; no false deletion | S |
| CORE-GAP-REJECT | Fault proxy drops one batch frame; replica refuses gap, reconnects, converges | S |
| CORE-KILL-DURING-APPLY | Kill replica during ApplyBatch repeatedly; integrity check: exactly-once application | S |
| CORE-KILL-GATEWAY-CAPTURE | Kill gateway between Capture and Committed; Helper resends; no duplicate journal rows | S |
| CORE-HELPER-RESTART | Kill SimApplePeer; gateway marks sources degraded, peer reconnects with checkpoints; no rescan storms, no deletions | S |
| HELPER-RESTART-REAL | Kill the real Swift Helper mid-capture and mid-command; same guarantees; unfinished dispatch → indeterminate | R |
| HELPER-TCC-READY | Helper reports every permission state truthfully; stable signing keeps grants across rebuilds | R |
| CORE-RESTORE-EPOCH | Restore gateway backup; epoch rotates; replicas resnapshot; old command IDs lookup-only | S |
| CORE-DISK-FULL | Fill gateway disk; capture stops degraded; no corruption; recovers when freed | S |
| CORE-SLOW-CONSUMER | Replica stops acking; gateway disconnects it before memory bound; it resumes | S |
| CORE-SCOPE-LEAK | Reminders-only token on replica and gateway: no messages via query/search/SSE/MCP/blobs/commands | S P |
| CORE-SCOPE-CHANGE | Replica scope narrowed then widened: purge then backfill in new subscription revision | S |
| CORE-PAIR | Pair Ubuntu replica with code; replay/expired/wrong code rejected; revoke closes connection | S P |
| CORE-FORGED-DELEGATION | Replica forges delegated grants beyond its delegable set → rejected before effect | S |
| CORE-LOCALHOST-DEFENSE | No token, bad Host, foreign Origin → 401/403; nothing executed | S |
| CORE-CMD-IDEMPOTENT | Same command ID twice → one effect; same ID different args → conflict | S |
| CORE-CMD-LOST-RESPONSE | Response dropped; client looks up by ID; one effect | S P |
| CORE-CMD-CRASH-AMBIGUOUS | Helper crashes after effect before outcome → indeterminate; no redispatch; Resolve upgrades on evidence | S |
| CORE-CMD-EXPIRY | Command admitted, Helper paused past expiry → expired, never dispatched | S |
| CORE-CMD-CLOCK-JUMP | Gateway wall clock jumps ±1 h; expiry uses monotonic timers; skewed expires_at rejected | S |
| CORE-CMD-OFFLINE | Replica disconnected: command fails fast unavailable | S P |
| CORE-CMD-CONFLICT | Two replicas edit same reminder with same expected_version: one wins, other gets conflict with current | S P |
| CORE-CMD-READ-YOUR-WRITE | Command status commit_seq + replica Wait → read shows result | S P |
| CORE-REST-MCP-PARITY | The same command and read journeys through REST and MCP give identical results | S |
| CORE-CRED-LIFECYCLE | Keychain and LinuxSecretStore: create, use, rotate, revoke | R P |
| CORE-UPGRADE | Rolling upgrade: newer replica with older gateway negotiates; unknown optional fields preserved | S |
| CORE-BLOB-INTERRUPT | Blob transfer cut mid-chunk; resumes from offset; digest verified | S |
| CORE-BLOB-CORRUPT | Corrupted chunk detected, refetched | S |
| CORE-BLOB-CAP | Replica cap reached: over_cap state, metadata still complete | S |
| CORE-BLOB-NO-DIGEST-AUTH | Requesting a blob by digest without a record grant → denied | S |
| CORE-BLOB-GC | Blob deleted only after no record/snapshot/command references and pins | S |
| CORE-DISCONNECT-DETECT | Gateway host goes away; replica reports disconnected + staleness within 15 s; reads continue | S P |
| CORE-BENCH | modernc SQLite+FTS5+WAL RSS/latency on the Air with 200k messages synthetic | S |

### 18.2 Messages (M1)

| ID | Scenario | Levels |
|---|---|---|
| MSG-HISTORY-IMPORT | Full import of local history; counts per chat equal Truth; live changes during import are not overwritten by backfill | S R |
| MSG-INCOMING | External self-message (via fixture to self) appears on replica < 1 s after chat.db row | S R P |
| MSG-SEND-SELF-TEXT | `messages.send` to self (iMessage): locally_observed then delivered; text equal; one row | S R P A |
| MSG-SEND-SELF-EMAIL | Send to own iCloud email handle | R |
| MSG-SEND-FILES | Text + 2 files: per-part outcomes; attachments become blobs on replica | S R P |
| MSG-SEND-GHOST | Ghost send (empty unjoined SMS row) → delivery_failed, not success | S (R if reproducible) |
| MSG-SEND-IDENTICAL-MANUAL | Identical text sent manually concurrently → indeterminate unless uniquely correlated | S |
| MSG-SEND-NEW-HANDLE | Send to a handle without an existing chat (own email when no chat) | S R |
| MSG-SEND-RCS-GROUP | Send into the existing Ben RCS/SMS group (if present) or create via `create_group` with Ben + self | R |
| MSG-REPLY | Threaded reply to own message part | S R |
| MSG-REACT | Add then remove tapback on own message; emoji reaction | S R |
| MSG-EDIT | Edit own iMessage; edit history visible; 6th edit rejected; >15 min rejected | S R |
| MSG-UNSEND | Unsend within 2 min; retracted visible; > 2 min rejected | S R |
| MSG-MARK-READ | Mark chat read/unread | S R |
| MSG-DELETE | Delete a test message and a test chat; absence published only after complete scan | S R |
| MSG-GROUP-CREATE | Create iMessage group (sim) / RCS group with Ben (real if capability proven) | S R |
| MSG-GROUP-MANAGE | Rename, add, remove, leave (iMessage group: sim; real where an iMessage group is available) | S |
| MSG-RECEIPTS | Delivered/read timestamps update on replica | S R |
| MSG-DECODE | Unicode (emoji, RTL, combining), long text, mentions, multipart, malformed body → undecoded not empty | S + decoder fixtures |
| MSG-DB-REPLACED | chat.db replaced (sim) → new generation, rescan, no false deletion | S |
| MSG-WATCHER-REARM | WAL file replaced/rotated; watcher rearms; polling fallback catches changes | S R |
| MSG-PERMISSION-LOSS | FDA revoked (sim) → permission_missing, no deletions | S |
| MSG-UI-INTERFERENCE | Human focus change mid-action → indeterminate/rejected; no duplicate action | S R |
| MSG-CAPABILITY-MATRIX | `sources.status` lists per-operation capability with reasons; unsupported ops reject before effect | S R |
| MSG-AGENT-TEXT-ME | Agent: "text me 'hello from the agent [ab-test]'" → exactly one self-message | A |
| MSG-AGENT-SUMMARIZE | Agent: "what did I last receive in the self chat" → correct from replica | A |

### 18.3 Reminders (M1)

| ID | Scenario | Levels |
|---|---|---|
| REM-CREATE-EXTERNAL | Created in Reminders (fixture) → replica < 1 s | S R P |
| REM-CRUD | create/update/complete/uncomplete/delete via API; Truth confirms values/absence | S R P (both) |
| REM-LIST-CREATE | create_list "Bridge Test" | S R |
| REM-CONFLICT | Stale expected_version → conflict with current | S R |
| REM-EXTERNAL-RACE | External edit between Prepare and Dispatch → conflict, no overwrite | S |
| REM-RECURRING | Recurring reminder: next incomplete instance only; completing advances | S R |
| REM-READONLY-LIST | Write to read-only list rejects before dispatch | S |
| REM-PERMISSION-LOSS | Reminders access revoked → permission_missing, no deletions | S |
| REM-AGENT | Agent: "remind me to buy milk tomorrow at 9 in Bridge Test" → correct reminder | A |

### 18.4 Calendar (M1)

| ID | Scenario | Levels |
|---|---|---|
| CAL-CREATE-EXTERNAL | Event created in Calendar (fixture) → replica < 1 s | S R P |
| CAL-CRUD | create/update/delete via API in "Bridge Test" | S R P (both) |
| CAL-RECUR-THIS | Update one occurrence (span this) → exception; others unchanged | S R |
| CAL-RECUR-FUTURE | Update future (span future) → split series | S R |
| CAL-RECUR-ALL-REJECT | span all rejects before dispatch | S |
| CAL-MOVED-OCCURRENCE | Moved occurrence keeps bridge ID via original occurrence date | S R |
| CAL-DST | Event across DST boundary keeps wall time/time zone | S R + time-math tests |
| CAL-ALLDAY | All-day event floating date preserved | S R |
| CAL-WINDOW-SLIDE | Day passes (sim clock): oldest events evicted (not deleted), new ones enter | S |
| CAL-ID-CHURN | Local identifier changes (sim) → same bridge ID via external alias | S |
| CAL-READONLY | Write to read-only calendar (subscribed/holidays) rejects | S R |
| CAL-PERMISSION-LOSS | Calendar access revoked → no deletions | S |
| CAL-AGENT | Agent: "put lunch with myself on Friday 12–13 in Bridge Test" | A |

### 18.5 Contacts (M2)

| ID | Scenario | Levels |
|---|---|---|
| CON-IMPORT | Full import equals Truth (field coverage per coverage.yaml) | S R |
| CON-EXTERNAL | Contact created/edited externally → replica < 1 s | S R P |
| CON-CRUD | create/update/delete Bridge Test contact via API | S R P (both) |
| CON-DROP-EVERYTHING | History drop-everything → rescan new generation, no false deletion | S |
| CON-TOKEN-CRASH | Crash between change fetch and token commit → no lost/duplicated change | S |
| CON-READONLY-CONTAINER | Update in read-only container rejects | S |
| CON-IMAGE | Contact photo becomes blob on replica | S R |
| CON-HANDLE-LINK | messages.handle links to contact_id | S R |
| CON-AGENT | Agent: "what's my own email address according to my card" | A |

### 18.6 Notes (M3)

| ID | Scenario | Levels |
|---|---|---|
| NOTE-IMPORT | Folders and notes equal Truth; locked notes title-only | S R |
| NOTE-EXTERNAL | Note created/edited externally → replica (< 1 s target; reported if Notes lags) | S R P |
| NOTE-CRUD | create/update(plain)/delete in Bridge Test folder | S R P (both) |
| NOTE-RICH-REJECT | Update of a note with checklist/table rejects before dispatch | S R |
| NOTE-DECODE | Protobuf fixtures: formatting, checklists, tables, attachments, malformed | decoder fixtures |
| NOTE-AGENT | Agent: "add 'eggs' to my Bridge Test shopping note" | A |

### 18.7 Usage (M4)

| ID | Scenario | Levels |
|---|---|---|
| USE-IMPORT | Devices and app_day estimates match an independent decode by AppleTruth (python-equivalent algorithm) | S R |
| USE-LOCK-CLIP | Locked/asleep intervals clipped from Mac totals (sim events; real lock via fixture `pmset displaysleepnow` is not allowed without Johan → sim + historical evidence) | S R |
| USE-REMOTE-LAG | Delayed remote batches update past days; freshness per device | S R |
| USE-SEGB-DECODE | SEGB fixtures incl. truncated/malformed records | decoder fixtures |
| USE-READONLY | Any write op on usage → unsupported | S |
| USE-AGENT | Agent: "how much time did I spend in Safari on my iPhone yesterday (estimate)" | A |

### 18.8 Packaging and agent suite (M5)

| ID | Scenario | Levels |
|---|---|---|
| PKG-INSTALL | Signed app installs; onboarding requests each permission; SMAppService registers | R |
| PKG-BACKGROUND | Logout-free background run after app quit; survives reboot login | R |
| PKG-UPGRADE | App upgrade keeps state, permissions, pairing | R |
| PKG-SLEEP | Sleep assertion held while serving; sleep/wake recovers with reconciliation | R |
| AGENT-SUITE | All `*-AGENT` scenarios 3× each, graded by Truth | A |

---

## 19. Code-level test allowance

Only where E2E cannot reach:
- Decoders: typedstream, message_summary_info, Notes protobuf, SEGB — fixtures captured from this Air and scrubbed (`lab/fixtures/`, `native/Tests/DecodersTests`).
- Time math: calendar window chunks, DST, occurrence identity, session pairing and clipping.
- Crash/race timing that can't be triggered from outside: State transaction boundaries (kill points injected through a lab-only build tag), dispatch guard fsync ordering.

If behaviour can be tested from outside, no unit test.

## 20. Quality gates (`make lint`, CI-equivalent run on the Air)

- **Zero comments** in Go and Swift source (including generated code and test code): `tools/lint` fails on any `//`, `/*`, `///` comment token (Go via `go/scanner`, Swift via a tokenizer that respects string literals). Build tags use file-name suffixes (`_lab.go`) instead of `//go:build` lines where possible; the only permitted exception is the `//go:build` constraint line when unavoidable, enumerated in the linter.
- Import boundaries per §5.
- `gofmt`, `go vet`, `swift build -Xswiftc -warnings-as-errors`.
- `features.yaml` checker passes.
- Secret scan: no phone numbers/emails/tokens matching patterns in tracked files except synthetic `+1555…`/`@example.com`.

## 21. Milestones

Each milestone proves its riskiest unknown first, starts every feature as a failing E2E scenario, and ends with reviews done, scenarios passing at assigned levels, pushed, PROGRESS.md truthful.

- **M0 — Harness and core at sim level.** Contracts, State, Operations, Replication, Blobs, Trust, Frontdoor (REST+MCP), Runtime basics; `bridge-lab` World/SimApplePeer/SimTruth/Scenarios/FeatureGraph; all CORE-* scenarios at S; CORE-BOOT/RESUME/PAIR/SCOPE-LEAK/CMD-OFFLINE/CMD-CONFLICT/LIVE-LATENCY at P with a Linux replica on Ubuntu. Riskiest first: State snapshot/ApplyBatch correctness and feed gap rules.
- **M1 — Messages, Reminders, Calendar end to end.** Riskiest first: (1) stable dev signing + TCC grant batch on the Air; (2) AppleScript send to self on 26.7.1; (3) Accessibility bubble targeting (react/edit/unsend). Then Swift Helper session, AgendaGate, sources, AppleTruth; scenarios sim → real → replica → agent.
- **M2 — Contacts.**
- **M3 — Notes.** Riskiest first: protobuf decode on this Air's NoteStore schema; AppleScript write round-trip.
- **M4 — Usage.** Riskiest first: lock/display evidence streams on 25G241.
- **M5 — Packaging and full agent suite.** Riskiest first: Developer ID availability for signing/notarization.

## 22. Known limits (truthful, kept current)

- Apple arrival lag (iPhone → Mac via iCloud) is outside the < 1 s target; Screen Time remote data lags ~20 h.
- Messages history = what's on the Mac; Messages in iCloud may keep only recent items locally.
- Calendar = ±2 years sliding window; recurrence "all" span unsupported.
- Recurring reminders: next incomplete instance only.
- Contacts notes unavailable without Apple's restricted entitlement; group management later.
- Notes: plain-text edits only; rich edits reject; locked notes title-only.
- Usage: estimates, not Screen Time Settings totals; parity unproven.
- iMessage group features verified at sim level and self-chat where possible; Ben is RCS/SMS.
- Sleep, logout, shutdown, FileVault pre-login are outages.
- Local file access on a replica bypasses API scopes; revocation cannot erase copied data.
- Losing `ledger.db` (not just `state.db`) loses the ability to recognize earlier command IDs; recovery requires an explicit operator acknowledgement.
- Notarized distribution requires an Apple Developer ID (status tracked in PROGRESS.md).
