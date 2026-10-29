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
23. Web app (W milestones)

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
| `messages.chat` | `service` (iMessage/SMS/RCS), `style` (direct/group), `display_name`, `participants[]` (handle ids), `is_archived`, `last_read_at`, `is_unread?` (explicit chat unread marker), `capabilities` (per-chat allowed actions), `filter` (`known|unknown`), `is_spam` (native Junk tier), `group_photo_attachment_id?`, `read_receipts` (`on|off|default`), `muted?` (known Hide Alerts boolean), `muted_until?`, `pinned_index?`, `deleted_at?`, `recoverable_until?` |
| `messages.handle` | `address` (phone/email), `service`, `contact_id?` |
| `messages.message` | `chat_id`, `sender_handle_id?`, `is_from_me`, `service?` (the message's own native service, e.g. `iMessage|SMS|RCS`; a chat's `service` can differ from its messages'), `sent_at`, `edit_until?` and `unsend_until?` (the Helper's action windows for the owner's own iMessages: 15 minutes and fewer than 5 edits, 2 minutes; absent when the action is impossible), `parts[]` (`{index, kind: text|attachment|genmoji, text?, attachment_id?, description?}`), `text` (joined plain text), `decode_status` (`decoded|partial|undecoded`), `raw_body_digest?`, `summary_decode_status?` and `summary_raw_digest?` (the edit/unsend summary blob's decode result), `reply_to_id?`, `thread_originator_id?`, `edited_at?`, `edit_history[]`, `retracted_parts[]`, `retracted_at?`, `delivered_at?`, `read_at?`, `error_code`, `effect?`, `kind` (`message|group_event|system`), `group_event?` (`kind`, raw `item_type/action_type/title`, `target_handle_id?`, `actor_handle_id?`), `balloon?` (§23.12; includes `decode_status` and `raw_payload_digest?` for failed archives), `audio` (`is_audio_message, played_at?, expirable, kept?, transcription?`), `schedule?` (`state, scheduled_for`), `subject?`, `delivery` (`quietly, notified_anyway`), `is_spam`, `has_unseen_mention`, `via_satellite`, `is_sos`, `formatting[]` (§23.12 UTF-16 ranges), `deleted_at?`, `recoverable_until?` |
| `messages.reaction` | `message_id`, `part_index`, `sender_handle_id?`, `is_from_me`, `kind` (love/like/dislike/laugh/emphasize/question/emoji/sticker), `emoji?`, `removed`, `sent_at` |
| `messages.attachment` | `message_id`, `filename`, `mime_type`, `uti`, `size`, `blob` (`{digest?, state: present|pending|offloaded|missing|over_cap|failed|disk_full}`), `derivatives` (`{thumb|display|video|poster|audio|live: {digest, mime, size, state, width?, height?}}`), `media` (`{width?, height?, duration_ms?, thumbhash?, waveform?, live?}`; ThumbHash is base64, waveform is 64 peaks in 0–255; §23.7), `is_sticker`, `sticker?` (`target_message_id, part_index, x, y, scale, rotation`), `hidden`, `genmoji`, `description?` |
| `calendar.calendar` | `title`, `account`, `color`, `writable`, `type` |
| `calendar.event` | `calendar_id`, `series_id?`, `occurrence_date?` (original), `title`, `notes`, `location`, `url`, `start`, `end`, `all_day`, `time_zone`, `recurrence?` (rule on series), `is_exception`, `attendees[]` (read-only), `alarms[]`, `availability`, `status` |
| `reminders.list` | `title`, `account`, `color`, `writable` |
| `reminders.reminder` | `list_id`, `title`, `notes`, `url`, `due`, `due_all_day`, `start`, `priority`, `completed`, `completed_at`, `recurrence?`, `alarms[]` |
| `contacts.contact` | names (given/middle/family/nickname/prefix/suffix/phonetic), `organization`, `job_title`, `department`, `phones[]`, `emails[]`, `postal_addresses[]`, `urls[]`, `birthday?`, `dates[]`, `social_profiles[]`, `instant_messages[]`, `relations[]`, `image` (blob ref), `container_id`, `writable`, `note` (`unavailable` without entitlement) |
| `contacts.group` | `name`, `member_ids[]` (read-only in v1) |
| `notes.folder` | `title`, `account`, `parent_id?` |
| `notes.note` | `folder_id`, `title`, `plain_text` (inline when it fits), `html?`, `blocks?`, `body_blob?`, `body_status?`, `preview?`, `created_at`, `modified_at`, `locked`, `attachments[]`, `pinned?`, `tags?`, `content_status` (`complete|locked|offloaded|undecoded`), `editable` (`plain|none`); external body contract in `design/web/notes.md` §5 |
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

`Kind` is `read` (served locally on any machine), `query` (transient Helper reads routed through the gateway), or `command` (routed to CommandTarget). Per-source descriptor files register through `internal/contracts/operations.go`; Frontdoor generates REST routes, MCP tools and OpenAPI from it. Input/Output are Go types; JSON Schema for MCP/OpenAPI is derived by reflection plus explicit enums/descriptions in struct tags.

**Read operations** (all sources): `messages.handles` (batched handle names, initials, contact and avatar references; §23.12), `records.get`, `records.query` (collection + typed filters + order + cursor pagination), `search` (FTS over text fields within scope), `messages.thread` (chat history page with reactions/attachments inlined), `calendar.agenda` (time range), `reminders.open` (incomplete by list/due), `contacts.lookup` (legacy name/exact handle; additive `q` searches partial names, normalized phones and emails in deterministic name/ID order, default and maximum 25 for `q`, with positive `limit` capped at 25), `usage.summary` (device/day/app ranges), `sources.status`, `commands.status`, `commands.list`, `replica.status`, `blobs.get` (REST only streams bytes; MCP returns metadata and a local file path on the same machine).

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
- A Capture may carry a private `live` priority hint. The gateway fetches live variant handles before backfill handles and preserves arrival order within each lane. The hint is never journaled or replicated.
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

Focus status and pack qualification: [spec/focus.md](spec/focus.md).

Music catalog query: [Music](spec/music.md).

Music catalog, player controls and library capture: [Music](spec/music.md).

Photos capture, query and command coverage: [Photos](spec/photos.md).

Contacts Me-card and notes entitlement contract: [spec/contacts.md](spec/contacts.md).

Mail capture foundation and local body queries: [Mail](spec/mail.md).

Files source details: [files](spec/files.md).

Permission status and explicit requests: [permissions](spec/permissions.md).

Mac capture: [contract](spec/mac.md).
Phone call capture: [source contract](spec/phone.md).

Transient Helper queries and the query-with-effect rule: [query contract](spec/query.md).
Audio query slice: [inputs, bounds and remaining work](spec/audio.md).

Vision and PDF query slice: [inputs, bounds and remaining work](spec/vision.md).
Shared helpers for new sources: [SourceSupport](spec/source-support.md).

Relay service and enrollment: [G2-1 / G2-2](spec/relay.md).
OAuth client metadata: [fetch and lab contract](spec/oauth.md).
Source registration: [append-friendly registries](spec/sources.md).
Shortcut provisioning and read-only App Intents discovery: [signed builds, source format and bounded metadata inventory](spec/shortcuts.md). Clock: [pack qualification and limits](spec/clock.md).
Remote onboarding and public readiness: [G1-3 / G1-4 / G1-6](spec/remote.md).

Every source publishes `system.source` with state, coverage, freshness and capabilities, and exposes capabilities individually gated on OS build and proven behaviour. A capability is advertised only after its real-level scenario passed on this OS build (recorded in `features.yaml`); otherwise operations return `ErrUnsupported` with reason `capability_unproven`.

### 9.1 Messages

**Read path.** `~/Library/Messages/chat.db` opened `SQLITE_OPEN_READONLY` with WAL (needs FDA). Schema probed at open (column list hashed into the generation); missing columns become explicit coverage gaps, never crashes. Watch `chat.db`, `chat.db-wal`, `chat.db-shm` and the directory with a `DispatchSource`/FSEvents watcher, re-armed after file replacement (inode change), debounce 25 ms, plus a 2 s polling fallback when no events arrive while the WAL size changes, plus a complete reconciliation every 15 min and after wake.

**Change algorithm (F8).**
1. Inserts: `message.ROWID > lastRowID` (scan position, database-local).
2. Mutations: rows whose mutable columns changed: `date_delivered, date_read, is_delivered, is_read, date_edited, date_retracted, message_summary_info, error, is_finished, item_type, attributedBody length` — detected by a bounded rescan of the most recent N=2,000 messages plus any message referenced by new rows (`associated_message_guid`, `reply_to_guid`, `thread_originator_guid`) on every change event, plus the periodic complete reconciliation which hashes every row.
3. Recoverable joins are followed on every change (including restoration outside the recent-row window); attachment linkage and Genmoji metadata are included in each message fingerprint. Chats, handles, chat membership (`chat_handle_join`), attachments (`message_attachment_join`, `attachment.transfer_state`): full-table hash diff on change (tables are small) and on reconciliation.
4. Deletions: a message/chat missing from a **complete** reconciliation scan of a chat is published absent; `deleted_messages`/`sync_deleted_*` tables are used as corroborating evidence.
5. Database replacement (inode or schema hash change) starts a new generation with a full rescan.

**Identity.** GUIDs (`message.guid`, `chat.guid`, `attachment.guid`) are native keys; ROWIDs are scan positions only.

**Decoding (F9).** `attributedBody` typedstream is decoded by `Decoders.TypedStream` (ported from the format documented by imessage-exporter, written fresh; no GPL code copied) into parts: text runs, attachment placeholders (`__kIMFileTransferGUIDAttributeName`), mentions, part indices. `message_summary_info` (binary plist) yields edit history and retracted parts. Undecodable payloads keep `decode_status=undecoded` and `raw_body_digest`; they never become empty text. Reactions: rows with `associated_message_type` 2000–2007 (add) / 3000–3007 (remove) plus `associated_message_emoji`, target `p:<part>/<guid>`.

**Group event normalization (W2).** `group_event` retains its raw numeric fields and adds `kind` (`member_added|member_removed|phone_changed|rename|member_left|photo_changed|photo_removed|unknown`), `target_handle_id?` and `actor_handle_id?`. The numeric format follows the [format author's documented mapping](https://github.com/ReagentX/imessage-exporter/blob/develop/imessage-database/src/tables/messages/models/group_action.rs); unmapped events remain `unknown`. Synthetic decoder checks establish mapping behavior; this does not claim a fresh real GUI proof of group management.

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
| `delete_message` / `delete_chat` | Accessibility (context menu → Delete, confirm) | no active membership, with `deleted_at` from the recoverable join; physical absence only after a complete scan | retained recoverable records stay in the copy; test-created data only in tests |
| `create_group` | Accessibility (new message, multiple recipients, send first message) | new chat with style group and participants | capability proven separately (F10) |
| `rename_group`, `add_member`, `remove_member`, `leave_group` | Accessibility (conversation details) | `group_action_type` / `group_title` rows, `chat_handle_join` changes | iMessage groups only; RCS rename if proven |

Conversation commands locate a unique sidebar row from the verified native target and snapshot all active members. Preparation requires Messages already in the foreground and does not open or activate a conversation. Dispatch rechecks the native read timestamp and member snapshot before the UI action. Lab conversation changes require entirely tagged fixtures; chat deletion additionally requires a private manifest proving this run created the chat after its recorded baseline. Real chat proofs and the UI self-test choose an unused discovered own handle and remain unverified for conversation-wide changes when no safe isolated chat can be created.

Tagged attachment-only cleanup requires a private run manifest, a post-baseline native message, only registered attachments, and verified own-handle peers. UI targeting uses exact registered filenames, distinguishes outgoing bubbles from incoming self echoes, and rejects multiple matches. It never falls back to deleting an unverified nearby bubble.

Every UI action records a before/after screenshot path in evidence (stored in the Helper support dir, excluded from git) where disk state alone cannot prove the visible effect.

**Riskiest unknowns (prove first in M1):** AppleScript send on macOS 26.7.1 — **proven 2026-10-17**: send to self via raw `any;-;` chat id produced `is_sent=1, is_delivered=1`; the outgoing row has `text` NULL (content only in `attributedBody`), and a self-chat send also produces an incoming echo row, so correlation must decode `attributedBody` and distinguish the echo. Still open: Accessibility tree of Messages 26 for bubble targeting with SIP on.

### 9.2 Calendar

- EventKit full access (`requestFullAccessToEvents`). Window: `[today-730d, today+730d]` in local time. Startup queries use chunks of ≤ 1 year; non-startup live full scans query the complete 1460-day window once, within EventKit’s 4-year limit (F12). Slides daily at local midnight: records leaving the window are published as evictions, not deletion: the journal change carries `op: evict` and the wire field `tombstone_reason: "window_evicted"` (optional; absent means an ordinary deletion). Replicas remove evicted records from the collection but must not report them as deleted to callers.
- Identity (F11): native key = `calendarItemIdentifier` + `occurrenceDate` (original) for occurrences; aliases also store `calendarItemExternalIdentifier` scoped by calendar/account. On local-identifier churn after full sync, the external identifier + occurrence date re-links to the same bridge ID.
- Series: each occurrence in the window is a `calendar.event`; the series rule lives on every occurrence (`recurrence`) with `series_id`; detached occurrences have `is_exception=true`.
- Change detection: Calendar database/WAL hints → debounce → refetch the complete window for live changes, diff against the last committed set → observations. Startup reads the nearest chunk (±30 days) first for latency. Fetching and per-event materialization yield to pending Reminders work. Native Calendar writes wait for the active capture to complete, precede queued scans, and hold the capture boundary through synchronous validation and save. Uncorroborated generic `EKEventStoreChanged` notifications never trigger a Calendar scan. Complete scan of the window produces `absent` with completion evidence. Periodic full verification every 15 min disables observation reuse and rebuilds every event field; changes that do not advance `lastModifiedDate` are therefore visible by the next verification capture (at most one period plus capture completion).
- Observation reuse was probed through public EventKit saves on macOS 26.7.1 (build 25G241): calendar, title, notes, URL, start/end, all-day, time zone, detached-occurrence and availability edits advanced `lastModifiedDate`, as did recurrence frequency, interval, count, end, weekday, month-day, year-day, year-week, month and position edits. Location and absolute/relative alarm edits can leave it unchanged. The fast-path stamp therefore combines identity, original occurrence, calendar/account, modification dates and floating zone with location and both alarm fields. Stamps live only in process memory; restarting the Helper discards them and publishes full. Uncertain or detached series rebuild. Status, derived identity/original-date/week-start fields and five attendee subfields were not independently writable through the probe; the 900-second verification disables reuse and bounds unprobed or remote-sync changes to one period plus capture completion.
- Interim Calendar capture limit: a delta larger than 240 KB is published in chunks to keep capture progressing; this case is not atomic and records `calendar_capture_split` with record and byte counts until transactional multi-frame IPC lands.
- Commands: create (calendar must be writable), update with span `this|future` (`EKSpan`), delete with span. `all` rejects. Expected version checked against a fresh native refetch (F7). Per-series serialization.
- A recurrence edited with This and Future Events starts counting at the split occurrence. A count-ended recurrence edited with This and Future Events is stored as the equivalent end date, because EventKit recounts from the original series. The inclusive bound is the start of the Nth requested occurrence in the event’s time zone, or its floating civil date for all-day events; Calendar.app displays “End Repeat: On Date”. Verification compares the occurrence set.
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

Notes extras: [source spec](spec/notes.md).

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
- **Command ledger in a separate database file `ledger.db`** (same directory, same protections): `commands(id, principal, request_hash, op, args, state, admission_era, created_at, expires_at, evidence, disposition, commit_seq, history)` and `dedupe(command_id, principal, request_hash, admission_era)`. It is retained independently of journal retention and mirror rebuilds, and **restore never replaces it** (C1, F4). Replicated `system.command` records carry the command ID, principal ID and an optional `awaiting_approval` boolean, scoped by the record source; they carry no arguments or command payload. Every command publication derives the marker from the current state, removing it in the same commit that resolves approval. Replica approval watchers re-read pending markers through a scoped index after an observer reset.
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
  3 agent approve: persist awaiting_approval; device approval resumes at admit; owner rejection finalizes rejected_by_owner
    admit: ledger row (admitted), per-object/series serialization slot
  4 conflict check: expected_version vs current State; mismatch → ErrConflict with current record
  5 Prepare over ApplePeer → persist prepared evidence (route, fingerprint, candidates)
  6 recheck authorization + server-UTC expiry (monotonic timer in-process) immediately before dispatch
  7 Dispatch → dispatched
  8 Outcome → final disposition; evidence persisted; commit_seq of the observing capture recorded
```

- Expiry: default and maximum TTL per operation (messages 60 s/5 min, others 5 min/1 h). Clock skew: a request whose `expires_at` is in the past or beyond max TTL relative to the gateway clock rejects; requests carry no client clock assumptions otherwise. Wake/restart: admitted-not-dispatched commands are re-evaluated; if expired → `expired`.
- `awaiting_approval` is persisted and nonfinal. Unanswered requests expire after 24 hours; `rejected_by_owner` is final. Command IDs remain idempotent across approval and restart; execution TTL starts at approval.
- Helper unavailable → `ErrUnavailable` immediately (fail fast).
- Lost response: the caller retries **with the original ID**, getting the stored status. Never automatic redispatch of `dispatched`/`indeterminate`.
- Correlation is conservative: evidence must match fingerprint **and** be after candidate IDs **and** be unique; two identical candidate rows (e.g. a manual identical send) → `indeterminate` with both candidates listed.
- Stronger later evidence (from `Resolve` or capture) may move `indeterminate → locally_observed/delivered/failed_no_effect`, appending to `history`.
- Status returns `commit_seq`; command submission and `commands.status` on a replica wait until its local cursor reaches the command cursor, so an observed result is immediately readable locally. A command from an older admission era remains lookup-only and skips the current-feed wait.
- Observed outcomes also persist `observed_record_ids`: concrete bridge record IDs resolved from the correlated native key in the active source generation and command epoch. Subsequent record updates do not invalidate that stable identity. For multipart sends, this compatibility field retains the primary message record; each attachment derives from its own part's message record. REST, MCP and forwarded replica status return the same IDs. Missing aliases leave IDs absent; other lookup failures also record `record_id_resolution_error` in outcome evidence/history. The command disposition alone sets the client label (observed or delivered is always Sent); missing IDs only prevent swapping a local echo for its captured row, and clients never match text, time or sequence to find it (`design/web/messages.md` §3.1, "Not sure if delivered"). Rejected, failed-no-effect and indeterminate outcomes never invent observed IDs.
- A single-part `delivery_failed` outcome receives `commit_seq` and `observed_record_ids` only when the same conservative correlation proves a concrete captured failed record. A ghost failure without a matching record remains `delivery_failed` without an ID or commit. The UI shows "Not Delivered" with Try Again disabled and explains that it cannot verify the message. A concrete failed ID enables explicit `messages.send` with `resend_of`; neither case automatically redispatches.
- A positive `messages.send` Helper outcome whose capture is not yet bound stays in flight for at most 10 s after that Outcome. On committed Messages captures, the gateway reruns read-only Resolve for the original command ID and applies the same fingerprint, capture and alias correlation; it never dispatches again. Receipt registration precedes notification. A delayed capture can therefore produce observed/delivered on the existing command watch without client action. The follow-up stops at its bound and leaves an unresolved outcome indeterminate.
- `messages.send` also exposes `parts: [{index, kind: text|attachment, upload_id?, record_id?, disposition}]` on command status, `cmd_status.command` and enriched `system.command` subscriptions. A nonempty text body precedes attachments in original request order. `upload_id` refers to that requested attachment by index. Each `record_id` resolves the Helper's correlated per-part `native_key` through the alias table under the same active-generation, epoch and capture rules as the primary record. Missing proof leaves that part unresolved; filenames and times never identify a part. The compatibility primary ID does not identify which part failed in a mixed outcome.
- Public per-part dispositions are `observed | delivered | delivery_failed | failed_no_effect | indeterminate`. Evidence only strengthens: an indeterminate part may become observed, delivered or definitively failed, and an observed part may become delivered or delivery_failed with fresh failure proof bound to the same captured record; a verified record ID is never cleared or replaced. The aggregate is observed only when every part is observed (delivered also proves observation); any definitive part failure makes it delivery_failed with per-part detail; otherwise it is indeterminate. New per-part evidence with the same aggregate appends an explicit `history` event with `transition: "parts_updated"` and advances `commit_seq` and the command cursor. Identical evidence is a no-op. Read-only reconciliation never redispatches.
- Multipart dispositions apply per part. Observed/delivered parts are Sent even without a resolvable ID. A `delivery_failed` part renders its captured failed row as Not Delivered without restoring a duplicate echo; `failed_no_effect` restores its echo because nothing was created. A concrete failed part may be resent with a fresh command ID and `resend_of` naming its own message, carrying only that part. A failed-no-effect part retries only its original content with a fresh command, without fabricating `resend_of`; an attachment is restaged because submitted ready upload IDs are consumed. Successful parts are never resent with it. Indeterminate parts follow §3.1 independently, including a read-only Check and the transcript notice when an unresolved candidate has already been released. No part is automatically redispatched.

### 11.3 Replica

`CommandTarget = RemoteGateway`: direct replicas use HTTPS POST to the gateway peer listener over mTLS; gateway-dialed and Remote replicas use framed command requests over their authenticated feed connection. Both carry a **delegated identity**: `{replica_id, local_principal, local_grants, local_level?, command_id, request_hash, iat}` signed with the replica's device key. The gateway verifies the replica certificate, signature freshness (≤ 60 s), that `local_grants ⊆ replica's delegable grants`, and authorizes against them. Disconnected → `ErrUnavailable` immediately. Response lost → status lookup by the same ID.

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
R→G UploadControl(signed begin|status|prepare|commit|abandon)  G→R CommandResponse
R→G BlobChunk(binary, after prepare)  G→R CommandResponse(chunkID, upload status)
```

**Capture atomicity on replicas.** A capture commits atomically on the gateway (§7.3), and a replica applies it atomically too. A `Batch` ends only at a capture (commit) boundary: the `through` sequence is the last change of a capture, so a batch carries whole captures. When the next capture alone exceeds the change or byte cap, it streams as consecutive `Batch` frames flagged `partial: true`, ending with one flagged `partial: false`. The replica stages the parts on disk and streams them through one transaction on the final part, then acks that cursor. Partial acknowledgments return byte credit without a committed cursor; abandoned staging is discarded before Hello, on close and on reset. Replica reads, read models and live deltas never expose a partial capture. Resume after a partial transfer restarts at the capture's first change. CORE-REPLICA-CAPTURE-ATOMIC covers this. The replica offers `capture_atomic` in Hello’s `features` list and the gateway echoes acceptance in Welcome. Only negotiated peers receive capture-aligned batches and `partial` parts. A replica that omits the feature receives legacy count/byte cuts without `partial`; a newer replica talking to an older gateway accepts legacy batches when Welcome does not confirm the feature.

On upgrade, retained history is conservatively coalesced through one commit marker at the current head. Journal pruning cannot advance until the retention floor passes that marker, so retained journal storage can grow for one retention period after upgrade. The backlog streams as disk-staged partials without exposing intermediate state.

**Upload direction.** Replica uploads reuse the same 64-byte blob header and at most 1 MiB binary payload in the replica→gateway direction, in either connection direction. Binary chunks use the bulk writer lane; live metadata and acknowledgments preempt queued bulk chunks. JSON metadata retains the 256 KiB limit. Small `UploadControl` requests use the existing paired-key delegated identity proof (≤ 60 s), current `messages:write` scope and the same response channel as commands. Begin supplies the full size and mandatory SHA-256 digest. Prepare binds the chunk ID, upload ID, digest and expected offset to the authenticated feed session, paired certificate and delegated owner for at most 30 s. A delegated principal revoked after prepare may still land bytes for up to 30 s; bytes alone have no effect, and commit requires a fresh signed control. Each prepared slot admits one binary chunk; an unprepared/replayed chunk closes the connection. Accepted offsets persist across reconnects under the same paired identity. Commit requires the digest and hashes all received bytes before readiness; status and abandon cannot extend the lease. No referencing command is admitted before readiness (§23.13).

**Remote authentication (`relay/1`).** Before `feed/1` Hello, a Remote connection proves both application identities independently of its public TLS certificate. The replica sends `AuthHello{nonce, certificate}`; the gateway sends `AuthChallenge{nonce, certificate, signature}`; the replica sends `AuthProof{signature}`; the gateway sends `Authenticated{principal, signature}`. Both nonces are independently random 256-bit values. Each handshake has a five-second deadline and an 8 KiB frame limit. The replica pins the gateway identity certificate fingerprint from pairing; the gateway authenticates the replica certificate against the current paired principal. No feed data or commands precede authentication.

The Ed25519 signature input is `apple-bridge-relay/1` plus NUL, a role (`gateway`, `replica`, or `accepted`) plus NUL, the 32-byte replica nonce, the 32-byte gateway nonce, then four SHA-256 digests: replica certificate DER, gateway certificate DER, pairing ID (empty for paired connections), and principal JSON bytes (empty except for `accepted`). The principal JSON is carried verbatim in the final frame. A new connection always requires a new server nonce, so a captured AuthProof cannot authenticate another connection. Wrong pins, expired identities, unpaired keys and Hello before authentication reject. After authentication, the existing feed uses the paired principal and its certificate, including the same visibility, subscription revision and ≤ 1 s revocation watcher.

**Connection direction.** Networks are often one-way (NAT, campus Wi-Fi, ACLs the user can't change). Two directions share one protocol:
- *Replica dials* (default): the replica opens the WebSocket to the gateway's peer listener. With a Remote enrollment it instead dials `wss://<public-host>/v1/feed` using publicly trusted TLS and then `relay/1`; snapshot, blobs, subscriptions and delegated command frames use that same connection.
- *Gateway dials*: for replicas the gateway can reach but which can't reach the gateway, pairing records `dial: <replica address>`; the replica runs `bridge start --role replica --listen <addr>` with an mTLS listener, and the gateway keeps an outbound connection to it (reconnect with backoff). After TLS (both certificates pinned exactly as in pairing) the replica still sends `Hello` first and everything above the transport is identical, including command forwarding, which travels back over the same connection as framed requests instead of a separate HTTPS call.
Direction is a pairing property, chosen at `bridge pair --dial-replica <addr>` or `bridge pair --via-remote`; it never changes trust, scope or feed semantics.

**Fresh delegated-agent approval.** A product replica offers `delegation_protocol: "delegation/1"` in Hello; Welcome confirms support. After Welcome is acknowledged, the gateway can send `DelegationRequest` with a request ID and `{principal, command_id, request_hash, challenge}`. The challenge is a fresh 32-byte random nonce encoded as hexadecimal. The replica reloads the current local agent, intersects its grants with the enrollment scope, and returns a `DelegationResponse` containing the paired-key signature over the normal delegation plus `challenge`. The gateway verifies the same paired certificate, exact principal, command ID, request hash and challenge, current replica scope, level and ≤ 60 s freshness. An ordinary command proof has no challenge and cannot satisfy this recheck. A missing or disconnected negotiated session leaves approval pending. A revoked principal or narrowed scope rejects before dispatch. Control requests are limited to 16 pending per session and 5 s; raw auth probes do not register as product delegation sessions. Transport direction does not change this protocol.

Rules (F2, F3, D4):
- `(after, through]` covers every sequence, including changes filtered out by scope; empty batches advance watermarks.
- Resume requires same epoch and subscription revision and `retentionFloor ≤ cursor.sequence ≤ head`; otherwise snapshot.
- Snapshot at S, then retained S+1.. replay, then live — from the same journal iterator so there is no gap at the catch-up→live transition.
- Interrupted snapshot staging is discarded unless resumed within its lease.
- Credits: 4 MiB outstanding metadata; replica acks after each committed batch. A consumer that doesn't ack for 30 s or exceeds the server's 16 MiB send buffer is disconnected; on reconnect it resumes or resnapshots.
- Replicas never poll for data. They detect disconnection locally (no heartbeat for 15 s → `system.replica.state=disconnected`, reads continue with staleness reported).
- Scope change for a replica → new subscription revision → purge + snapshot of the new scope.
- Local SSE (`GET /v1/changes?after=`) reuses the same framing semantics for local subscribers, filtered by the caller's Visibility. An oversized capture resets local observers before any part is exposed (`event: reset` with `error: reset` for SSE, `error{code: reset}` for web/1); clients and internal RYW, status and approval watchers re-read the committed snapshot instead of consuming feed parts.

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
- **Remote pairing codes.** `bridge pair --via-remote` requires Remote to be on and rejects direct peer overrides or `--dial-replica`. `bridge join <code>` selects the encoded transport. A Remote code carries `remote: true`, the public HTTPS root URL including its selected port, and the same pinned gateway identity fingerprint. The pairing ID is bound into all `relay/1` signatures; `AuthProof.proof` carries the existing HMAC over the replica certificate. Only after verifying both signatures and the one-time proof does the gateway return the signed replica ID and scope. Enrollment persists the Remote marker and URL, so transport selection cannot silently fall back to an unpinned endpoint. Pair expiry, five attempts and single use remain the same as direct pairing.
- **Replica agent delegation.** Replica-local `agent:` identities carry their signed `read`, `approve` or `full` level. The gateway names the actor `replica:<id>/agent:<id>`, applies both the local grants and current paired replica scope, and records that identity in command activity. Read agents cannot issue commands. Approve agents create gateway approvals; owner approval requires the fresh signed recheck of §12, including local revocation and current paired scope. The native Helper accepts this composite agent identity with a gateway-selected interactive origin after those level and consent checks; malformed replica-agent identifiers are rejected at the IPC boundary.
- **Remote device identity.** `relay/1` (§12) uses the existing paired Ed25519 key and pinned certificate identity at the application layer. Publicly trusted TLS is required in addition to the application pin; a public TLS certificate alone never authenticates a gateway or replica.
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
- `doctor`: permissions, disk encryption (FileVault on macOS, LUKS check on Linux → warning), file modes, clock skew vs gateway, versions, and `shape`: counts of stored Messages records that drift from the §6.2 field shapes (`<collection>.<field>: unknown_field|missing_field|wrong_type|unexpected_null`), checked by the one shape module in `internal/contracts` against which the Helper, SimApplePeer and the demo are held.
- `restore <backup>`: install backup read-only; rotate epoch and admission era; then serve.
- Sleep: gateway holds an `IOPMAssertion`-equivalent via the Helper (`ProcessInfo.beginActivity(.idleSystemSleepDisabled)`) while serving. Logout/sleep/shutdown/FileVault pre-login are outages; replicas detect them independently.
- **Packaging (M5):** `Apple Bridge.app` contains the Helper (main executable, UI for onboarding), `bridge` in `Contents/MacOS`, LaunchAgent plist registered with `SMAppService.agent`. Signed app with hardened runtime; notarization is optional, not required. Open-source users may build from source or use right-click → Open for an unnotarized build. Foreground onboarding requests FDA (opens System Settings pane), Automation (Messages, System Events), Accessibility, Contacts, full Calendar, Reminders. Dev builds use a stable self-signed code-signing identity created on the Air so TCC grants survive rebuilds.
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
| CORE-GATEWAY-DIALS | Gateway-initiated connection to a listening replica: snapshot, live, resume after partition, command forwarding over the same connection, revoke | S P |
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
| PKG-INSTALL | Signed app installs in ~/Applications; onboarding permission status is readable; SMAppService registers and launchctl plus /v1/health prove the gateway is running | R |
| PKG-BACKGROUND | Gateway serves after foreground app quits; launchd restarts a killed gateway and it unlocks its dedicated Keychain; RunAtLoad registers startup at login (reboot not exercised) | R |
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
- **M5 — Packaging and full agent suite.** Riskiest first: signed app, onboarding and background service. Notarization is optional.

## 22. Known limits (truthful, kept current)

- Messages W2 native gaps on this Air: pinned order and mute expiry have no proven non-UI source; both remain null with `coverage.missing`. Audio transcription and an explicit Keep decision are not exposed by the observed schema; `kept` remains null rather than inferring Keep from the expirable flag. `schedule.state` preserves the native integer as `native_<n>` until state meanings are independently proven; `scheduled_for` is the message's native date. Recoverable tables expose `delete_date` but no recovery deadline; `deleted_at` is captured and `recoverable_until` remains null with coverage, rather than claiming an exact retention period. Sticker geometry is emitted only where all four stored transform components exist; older sticker records without placement retain `is_sticker`. Unreadable/malformed balloon archives remain typed placeholders with a digest and decode status. Missing native samples for a kind are verified with synthetic fixtures and do not establish real occurrence of that kind.
- Apple arrival lag (iPhone → Mac via iCloud) is outside the < 1 s target; Screen Time remote data lags ~20 h.
- Messages history = what's on the Mac; Messages in iCloud may keep only recent items locally.
- Calendar = ±2 years sliding window; recurrence "all" span unsupported.
- Recurring reminders: next incomplete instance only.
- Contacts notes unavailable without Apple's restricted entitlement; group management later.
- Notes: plain-text edits only; rich edits reject; locked notes title-only.
- Usage: estimates, not Screen Time Settings totals; parity unproven.
- iMessage group features verified at sim level and self-chat where possible; Ben is RCS/SMS.
- Sleep, logout, shutdown, FileVault pre-login are outages.
- Replicas without direct gateway reachability can pair through Remote and use its public Funnel URL (§12, §23.5). Remote must already be enabled on the gateway.
- Local file access on a replica bypasses API scopes; revocation cannot erase copied data.
- Losing `ledger.db` (not just `state.db`) loses the ability to recognize earlier command IDs; recovery requires an explicit operator acknowledgement.
- Packaged gateways use a dedicated `Apple Bridge.keychain-db` in their 0700 application support directory. Installation creates a random 256-bit password in a 0600 `keychain-password` file; the gateway unlocks this Keychain before credential operations, including after login or a crash. Credentials remain Keychain items, with `/usr/bin/security` as their trusted accessor. This permits unattended operation while the login Keychain is locked; possession of the support directory permits unlocking credentials, so it does not protect against another process running as the same user. The user Keychain search list is preserved. Unpackaged CLI use retains the login Keychain default; isolated lab worlds explicitly select file credentials.
- Notarization is optional, not required. Users may build from source or use right-click → Open; optional notarization requires an Apple Developer ID.

---

## 23. Web app

Decisions: `DECISIONS.md` (2026-10-20). Product spec per app: `design/web/<app>.md` (Messages: `design/web/messages.md`). This section is the binding architecture for the web milestones W1+.

### 23.1 Modules

| Module | Path | Owns | Depends on |
|---|---|---|---|
| Web | `internal/web/` | Browser front door: static app, `/web/v1` API, device sessions, passkeys, live views (`web/1`), agent management and OAuth pages, blob variants | Operations, Replication (journal iterator), Blobs, Trust, Contracts, `webui` |
| Remote | `internal/remote/` | Embedded Tailscale node (tsnet), Funnel listener, public routing to Web / Frontdoor / Replication | handlers injected by Runtime; Trust (state secrets) |
| WebUI | `web/` (TypeScript) + `web/embed.go` (package `webui`) | SolidJS app; built assets embedded into `bridge` | — (only `internal/web` imports `webui`) |
| Read models | `internal/state/` (tables) + `internal/operations/` (ops) | Derived per-view indexes and the `messages.chats` / `messages.thread` / `messages.collection` reads | State, Trust, Contracts |
| MediaDerivatives | `native/Sources/MediaDerivatives/` | Browser-safe variants of attachment blobs (Swift, system frameworks only) | AppleSession blob path |
| Demo | `lab/demo/` + `bridge-lab demo` | Deterministic synthetic dataset on a sim world, serving the web UI | lab World, SimApplePeer |

Frontdoor (§15) keeps REST/MCP semantics; Remote reuses its handler with a public-host policy (§23.5). Runtime reuses one Web handler across local and Remote listeners so sign-in notifications reach both sets of live sockets; Remote owns the public HTTPS Host policy. `bridge web --demo` from DECISIONS is realized as `bridge-lab demo`, because the product binary never imports `lab/` (§5).

New dependencies (approved): `tailscale.com` (tsnet, BSD-3), `github.com/go-webauthn/webauthn` (BSD-3). Frontend runtime: `solid-js` only. Frontend dev: `vite`, `vite-plugin-solid`, `typescript`, `@playwright/test`. Node and pnpm are build-time only (Homebrew on the Air); release binaries embed the built assets.

### 23.2 Web front door

Same origin for everything. Served by the gateway and by replicas (any instance), locally on the Frontdoor listener's host under `/` and publicly through Remote. Every request, including TLS, must match an explicit host allow-list: `localhost:<local-port>`, `127.0.0.1:<local-port>`, or the configured public HTTPS host including its selected port. Origin and relying-party ID derive only from that accepted value.

**Static.** `GET /` and app routes return `index.html` (`Cache-Control: no-store`). `/assets/*` are content-hashed (`public, max-age=31536000, immutable`). `/sw.js` and `/manifest.webmanifest` are `no-cache`. If `web/dist/index.html` is absent (assets not built), `/` returns a plain page saying `make web` is needed.

**API `/web/v1`** (JSON; cookie auth; every state-changing request and the WebSocket upgrade require `Origin` equal to the serving origin):

| Route | Purpose |
|---|---|
| `GET session` | `{device_id, kind: trusted|public, expires_at, cache: persistent|memory}` or 401 |
| `POST enroll/options`, `POST enroll/finish` | First passkey, authorized by a one-time enroll token |
| `POST login/options`, `POST login/finish` | Discoverable-credential sign-in (user verification required); body picks `kind` |
| `POST logout` | Ends this session |
| `GET devices`, `DELETE devices/{id}` | Sessions list and revoke (revoked sockets close within 1 s; the client wipes its cache on `session_revoked`) |
| `GET passkeys`, `POST passkeys/options`, `POST passkeys/finish`, `DELETE passkeys/{id}` | Manage passkeys from a signed-in session |
| `GET agents`, `POST agents`, `DELETE agents/{id}`, `GET activity` | Agent tokens and activity log (§23.6) |
| `GET approvals`, `POST approvals/{command_id}` | Approve or reject agent commands |
| `GET blob/{digest}?record=<id>&variant=<v>` | Bytes with Range, `ETag` = digest, `private, max-age=31536000, immutable`; `variant` ∈ `original|thumb|display|video|poster|audio|live` |
| `POST upload` | Stage a blob for sending (streamed, ≤ 100 MiB) |
| `POST handles` | Read-only `messages.handles{ids[]≤500}` for frame-batched names and contact avatars |
| `POST lookup` | Device-session recipient lookup through Operations `contacts.lookup`; projected `{results:[{contact_id,display_name,handles:[{value,kind:phone|email,label}]}],capability}` only |
| `GET sources` | Read-only `sources.status` through Operations with Scope and Visibility; source states, per-operation proven/unproven capabilities and reasons, and each `system.source` record ID for live proof subscriptions |
| `GET live` | WebSocket, protocol `web/1` (§23.3) |

**Recipient lookup.** `POST lookup` accepts `{q,limit?}`: trim `q`, then require 1–64 Unicode characters; `limit` defaults to 8 and accepts 1–20. `contacts:read` and the device principal’s Visibility apply. Without that scope, return `{results:[],capability:"unavailable"}` so raw handle entry remains available. Responses use `Cache-Control: no-store`. Limit each session to 20 lookups per second; excess requests return 429 with `Retry-After`. The client debounces 80 ms, aborts stale requests and displays only the latest query’s results. Agents keep using the existing MCP/REST `contacts.lookup` path; `/web/v1` remains device-cookie authenticated.

**Source discovery freshness.** Responses use `Cache-Control: no-store` and an increasing `X-Bridge-Source-Sequence` per serving process. Discovery supplies source IDs; actions require fresh live proven coverage. JSON API routes bypass the service worker; the approved trusted blob thumbnail cache remains separate.

**Upload locality.** Upload IDs are staged on the serving instance and owned by the device principal. A replica forwards verified bytes through the feed bulk lane, and reports readiness only after gateway verification (§23.13). Gateway uploads retain their local staging path.

**Ceremony JSON.** Options requests accept `{token?, kind?: trusted|public}` (`token` is required for enrollment, `kind` defaults to trusted). Options responses contain the standard WebAuthn `publicKey` object and `ceremony_id`. Finish requests contain `{ceremony_id, credential}`, where `credential` is the WebAuthn credential JSON (base64url binary fields). The server binds the single-use ceremony to its flow, serving origin, chosen session kind and, for additional passkeys, the signed-in device. Ceremonies expire after five minutes. Passkey/device list responses are `{passkeys: [...]}` / `{devices: [...]}`. Passkey list rows expose only `id`, `rp_id`, `created_at`. Device IDs include the `device:` prefix. `bridge web enroll --host <origin>` defaults to `http://localhost:7780`; this implementation prints the enrollment URL only, avoiding another QR dependency.

**Sessions.**
- **Principal:** `device:<id>` in Trust, with the owner's full grants.
- **Cookie:** `__Host-ab_session` (Secure; HttpOnly; SameSite=Strict; Path=/), an opaque 256-bit value stored hashed. Only plain HTTP on a pinned `localhost:<port>` or `127.0.0.1:<port>` uses `ab_session_local` without Secure, retaining HttpOnly, SameSite=Strict and Path=/. Public and TLS requests never accept the local cookie; plain HTTP never accepts the `__Host-` cookie. Refresh and logout use the serving origin’s cookie.
- **Trusted:** 30 days sliding, 90 days absolute.
- **Public:** session cookie with no Max-Age, 30 minutes idle and 12 hours absolute, with `cache: memory`, meaning nothing goes to IndexedDB, Cache Storage or the service worker.

**Passkeys.**
- **Last credential:** deleting the final passkey for a relying-party ID returns 409; deleting a nonfinal key remains allowed.
- **Relying-party ID:** the serving hostname: the Funnel name publicly, `localhost` locally. Each relying-party ID has its own credentials.
- **First passkey:** registered only through an enroll token from `bridge web enroll`, which `bridge setup` calls. The token is 160-bit, valid 10 minutes, single use, and printed as a URL plus QR (`https://<host>/enroll#<token>`).
- **Later passkeys:** added from signed-in sessions.
- **Hostname change:** if the public hostname changes, passkeys must be enrolled again (documented).

**Protection.**
- **Rate limits:** sign-in and enroll at 10 per minute and 100 per day per client IP (IPv6 grouped by /64), with failure backoff per IP escalating to 60 s. Recent failures from another IP never apply to a clean IP; only a high global ceiling of 600 auth requests per minute protects resources. API at 50 requests per second per session.
- **Headers:**
  - CSP `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`
  - `Referrer-Policy: no-referrer`
  - `X-Content-Type-Options: nosniff`
  - `Permissions-Policy: camera=(self), microphone=(self), geolocation=(self)`
  - HSTS on HTTPS
- **Revoke everything:** `bridge web lock` revokes every device session.
- **Sign-in events:** each successful sign-in emits a `signin` event to every other live session.

**Interactive principal.** Commands submitted by a `device:` principal through `web/1` are **interactive**. They carry `origin: interactive` into Operations and across the Apple seam. The Helper's lab gates (`[ab-test]` tag, own-handle routes) apply to lab and agent principals only. Interactive commands still need the operation's capability proof on this OS build (W3 self-test). Client code submits commands only from input-event handlers (enforced by a Playwright check that no command is sent without a preceding trusted input event).

### 23.3 Live protocol `web/1`

WebSocket `GET /web/v1/live`, JSON text frames `{"t": …}`. The logical protocol is `web/1`. The `Sec-WebSocket-Protocol` wire token is `web.1`, because `/` is not a legal HTTP token (WHATWG WebSockets, constructor step 9). Client, server and lab checks offer and select `web.1`.

```text
C→S hello{v:1}                                   S→C welcome{session, head}
C→S sub{id, view, args}                          S→C snap{sub, data, cursor} then delta{sub, changes[], cursor}…
C→S unsub{id}
C→S cmd{id, op, args, expected_version?}         S→C ack{id, status} then cmd_status{id, status}… until final
C→S resolve{id}                                  S→C cmd_status{id, status, command, throttled?}
C→S ping                                         S→C pong; S→C beat{head} every 15 s
S→C signin{device, at} · approval{command} · session_revoked · error{code, message}
```

**Views:**
- `chats{filter: all|known|unknown|unread|deleted|spam, participants?}`: ChatSummary rows (§23.4) and order keys. Deltas upsert or remove rows. The conversation list keeps one stable paginated `all` view and starts fixed `deleted` and `spam` views after paint. All/Known/Unknown/Unread arrays are precomputed from the all-view rows and updated by live deltas; selecting a filter only switches arrays, without a new subscription or snapshot reply. Server filters remain the membership oracle for all six views, including Contacts-hidden principals.
- `thread{chat_id, thread_originator_id?, anchor: latest|oldest|unread|<message_id>, before?, after?}`: a window of ThreadItems. Deltas update items inside the window and report `newer`/`older` counts outside it.
- `record{collection, id}`.
- `collection{chat_id, kind}`.
- `search{text?, kind?, from_handle_id?, from_contact_id?, unread?, after?, limit?}`: scoped message full-text search with filters applied before pagination (§23.4).

**View payloads.** The WebSocket upgrade offers subprotocol `web.1` for logical protocol `web/1`. `record` snapshots contain the `records.get` result (`records`, `cursor`); record deltas contain the same Change rows as SSE. For an owned `system.command` record, Web enriches the record data from `commands.status` so reconnecting clients can reconcile their command without resubmitting it. Read-model snapshots contain the corresponding operation result. Read-model delta changes are `{op: upsert, id, row, order}` or `{op: remove, id}`; `order` is the zero-based position. A read-model delta also carries the ordered ID array `order` and pagination fields in `page`. Thread subscriptions re-evaluate their requested read window: `anchor: latest` follows the tail, `anchor: oldest` follows the head, and explicit message anchors remain centered on that item. Thread snapshots include `data.newer` and `data.older`; thread deltas carry total `newer` and `older` counts outside the subscribed window, also emitted as a `{op: window, newer, older}` metadata change so row reducers update those counts even when no window item changed. A subscription may provide its resume `cursor` next to `args`; record subscriptions replay journal rows. Derived subscriptions take a fresh snapshot on reconnect because their previous window membership is client state. Unsupported views produce `error{code: unsupported}` until their Operations descriptors are available. `ack.status` and `cmd_status.status` are command-state strings; `cmd_status.command` contains the command ledger record, and rejected acknowledgments carry `error`. Socket overload and revocation close with code 1008 and reason `overloaded` / `session_revoked`.

**Rules:**
- **Check:** `resolve{id}` runs read-only `commands.status` with `resolve: true` for that principal-owned command only. It never submits or redispatches a command. Unknown and foreign IDs are hidden. At most one Resolve per command every 5 s and ten per device session in a rolling minute are admitted; excess requests return the stored `cmd_status` with `throttled: true`. Limits survive socket reconnects. Check also refreshes the owned command-record subscription, while reconnect alone only reads stored status and never requests Resolve automatically. Resolve errors include the requested command `id`; the client shows a human-readable Check failure on that command’s notice without changing its delivery disposition.
- **Deltas:** come from the State journal through the same iterator as local SSE (§12), filtered by Visibility and mapped to affected subscriptions.
- **Optimistic reconciliation:** `cmd_status.command` and enriched `system.command` record subscriptions carry `observed_record_ids` with `commit_seq` (§11.2). A local echo is swapped for its captured row only after its concrete captured IDs appear in a compatible view cursor at or beyond the command commit. Without IDs or a comparable cursor, new outgoing captured rows in that chat are held for at most 500 ms while the echo exists, then released, and the echo is suppressed. A definitive failure restores the echo as Not Delivered. An indeterminate outcome keeps the echo as "Not sure if delivered" unless a candidate row was released, in which case a transcript notice with a read-only Check replaces it. Labels follow the disposition, never the presence of IDs.
- **Budget:** journal commit to frame written ≤ 20 ms p95 on the gateway.
- **Reconnect:** the client resubscribes with its last cursor per view. The server replays from the journal when it can, otherwise sends a fresh `snap`.
- **Backpressure:** at most 4 MiB unsent per socket; beyond that the socket closes with `overloaded` and the client reconnects.
- **Command IDs:** generated by the client (ULID), so a resend after reconnect is idempotent (§11.2).

### 23.4 Read models

Operations (descriptors in `internal/contracts/operations.go`; available through REST, MCP and `web/1`; scope `messages:read`):

- `messages.chats{filter?: all|known|unknown|unread|deleted|spam|archived, participants?: handle[], limit≤200, after?}` → `{chats: ChatSummary[], next?}`. Ordered by `last_activity_at` desc, then id. Optional `participants` contains 1–32 phone/email handles and matches the exact normalized set (case-folded email and normalized phone; duplicates folded), never a subset or superset. It uses the handle-address and chat-membership indexes in one bounded lookup, applies Visibility, and binds pagination cursors to the normalized set. ChatSummary fields:
  - `chat`: the record
  - `title`: display name, or the participants' contact names joined
  - `avatar_handles`: up to 3 handle ids with `contact_id`
  - `known_sender`: boolean matching the Known Senders server filter: native known classification (including its default), or a non-deleted participant handle with a contact link. Derived from the already-loaded full participant handle set, including members beyond the three avatar handles. Contact visibility does not change this membership bit; it never exposes contact names or IDs.
  - `last_message`: `{id, sent_at, is_from_me, sender_handle_id?, service?, preview{group_event?, kind: text|attachments|reaction|group_event|unsent|edited, text?, attachment_kinds?, count?, reaction?}}`
  - `last_activity_at`, `unread_count`, `has_unseen_mention` (from captured message flags)
  - `muted` and `pinned_index`, nullable where native state is unavailable
  - `pin_preview`: `{message_id, text_preview, at} | null`, the latest unread incoming message with text bounded to 200 Unicode code points plus an ellipsis, maintained by the read model’s existing deltas. The UI reads this field directly and never pages through a thread to construct a pin bubble.
  - The client persists one atomic owner-scoped materialized snapshot per transport view, bounded to the first 2,000 rows and 2 MiB. Deltas mark persistence dirty; snapshot construction, UTF-8 encoding and size checks run together after paint in a coalesced background task at most once per 500 ms, with pagehide/hidden-page flushes. Per-page transport entries are neither persisted nor hydrated. Public sessions persist nothing and sign-out/revocation wipe the snapshots.
- `messages.thread{chat_id, thread_originator_id?, anchor?, before?, after?, limit≤200}` → `{items: ThreadItem[], before?, after?, unread_anchor?}`. ThreadItem is the message record plus:
  - `reactions[]`: current state per sender and part, with removals folded out
  - `attachments[]`: records including `derivatives` and `media`
  - `reply_count` for thread originators
  - Ordered by `sent_at`, then id. Cursors are stable under concurrent inserts.
  - Optional `thread_originator_id` restricts the view to that root and all messages whose originator matches it in the same chat. The root must exist in that chat. Visibility, anchors, the 200-item page bound and live deltas apply unchanged; outside-window counts are restricted to the selected thread. Filtered cursors bind both chat and originator and cannot be reused in another thread or an unfiltered view.
  - A composite `(chat_id, originator, sent_at, id)` index keeps distant replies readable without scanning the surrounding chat.
  - `anchor: oldest` returns the first bounded page in ascending order, symmetric to `latest` at the tail. Continue forward with `after`; `before` returns the preceding page. This anchor supports transcript ⌘↑ without fetching intervening history.
- `messages.search{text?, kind?: photos|links|documents|locations, from_handle_id?, from_contact_id?, unread?, after?, limit≤200}` → `{items: ThreadItem[], next?, cursor, coverage_notes}`. Text is a literal prefix full-text query over message content, link titles, app captions and visible attachment filenames, at most 512 characters. The version-4 FTS projection rebuilds existing version-3 stores through the existing bounded background migration. At least text or one filter is required. All filters apply before pagination. Results exclude deleted/retracted messages, deleted/archived chats and spam, ordered by message time descending then ID ascending, with visible attachments inlined. Sender handle and contact filters are mutually exclusive; contact lookup requires Contacts visibility. Seek cursors are bound to the normalized query, and source changes update matching live rows. The `search` live view carries this read model; conversation-title and contact sections use their existing scoped sources.
- `messages.collection{chat_id, kind: photos|videos|media|links|documents|locations|audio, after?, limit≤200}`. Links and locations return message records with `balloon.kind=link|find_my`; media kinds return attachment records. Hidden attachments and recoverable messages are excluded. `kind: media` combines photos and videos in the existing owning-message join, ordered by owning message `sent_at` descending, then attachment id ascending. Each attachment projects its owning `data.message_id` and `data.sent_at` without mutating the stored record. It shares collection visibility, limits, budgets and kind-scoped cursors; inserts newer than the cursor do not change older pages.

- `messages.handles{ids[]≤500}` → `{handles: [{handle_id, address, service, contact_id?, display_name, given_name?, family_name?, initials, avatar{digest, variant}?}], cursor}`. Duplicate IDs are folded and unknown IDs omitted. Contact names and avatars require contact visibility. Contact images use `variant=original` with the chosen thumbnail or original digest. Available as the `handles` live view as well as ordinary REST/MCP reads.

**Unread:** an explicitly marked-unread chat has a minimum summary unread count of one and belongs to the Unread filter. A message counts as unread when it is not from me, has `kind = message`, has `read_at` null, and is not recoverable. The default chat list excludes recoverable chats; the deleted filter also includes active chats containing recoverable messages. Known includes native known-sender chats and chats with a linked contact; spam selects native Junk chats or chats containing a spam message.

**Implementation:**
- Derived tables in `state.db` (`msg_by_chat`, `chat_summary`, `reaction_by_message`, `attachment_by_message`), updated in the same transaction as each record apply and rebuilt on snapshot install, on the gateway and replicas alike.
- They're never journaled and never authoritative.
- Visibility is applied at read time, as for every read.

**Budgets on the Air:**
- `messages.chats` first page ≤ 15 ms p95 at 500 chats.
- `messages.thread` page of 100 ≤ 10 ms p95 on a 170k-message store.

**Reminders read models (W5-STORE; scope `reminders:read`):**

- `reminders.lists{time_zone}` → `{smart: SmartTile[], lists: ReminderList[], tags: TagCount[], cursor}`.
  - `SmartTile`: `{id: today|scheduled|all|flagged|completed|assigned, count?, status?: unsupported}`; incomplete counts except Completed. Today includes overdue incomplete reminders and reminders due during the viewer's day. Scheduled has a due date and is incomplete. Flagged requires the native store flag. Assigned means assigned to the current user, never merely assigned to someone. Its predicate uses the source-verified `assignment.assignee.is_me` boolean; unknown current-user identity makes the Assigned tile `status=unsupported` with no count, just like an unsupported custom smart filter. Never infer identity from a display name. Each predicate has a synthetic-store scenario assertion.
  - `ReminderList`: `{list: Record, count?, status?: unsupported, sections: ReminderSection[], group_id: id|null, order, pinned}`. This is a flat sidebar projection; groups are list records with `is_group=true`, and each child names its group in `group_id`. Return pinned entries first in native pin order, then native display order, with record id only as a tie breaker. The UI preserves the returned order and uses `group_id` for disclosure folders without sorting. Custom smart lists get a count from a supported decoded filter, or `status=unsupported` with the count omitted.
  - `TagCount`: `{name, count}` counts incomplete visible reminders once per tag. If tag coverage is unavailable for a visible incomplete reminder, omit the tag facet rather than publish partial counts.
- `reminders.items{time_zone, list?: id, smart?: today|scheduled|all|flagged|completed|assigned, tag?: name, show_completed?: false, sort?: manual|due|created|priority|title}` → `{sections: ReminderSectionItems[], count, cursor}`. Exactly one selector is required. Unknown selectors/sorts reject as invalid.
  - `ReminderSectionItems`: `{section: ReminderSection, items: ReminderItem[]}`. Empty native sections remain visible for a list selector. Scheduled groups by due day (`kind=date`); All and Completed group by list (`kind=list`, with `list_id`). Today uses one `kind=none` section until the current OS daypart grouping is verified; verified grouping may use Overdue / Morning / Afternoon / Tonight (`kind=daypart`).
  - `ReminderItem`: `{reminder: Record, subtasks: ReminderItem[], attachments: Record[]}`. Subtasks nest only under a selected, visible parent in the same section; unmatched children remain roots so generated date sections retain the child's due day. Cycles are broken deterministically. Each reminder contributes once to `count`. Attachment ownership selection uses one JSON-array SQL parameter via `json_each`, including selections larger than SQLite's variable limit.
  - `ReminderSection`: `{kind: store|list|date|daypart|none, id, title, order, list_id?}`; `kind=none` is the unsectioned bucket. Section membership and ordering require a supported store decoder; unavailable membership places reminders in `kind=none`. Both operations require an IANA `time_zone` from the viewer and reject invalid zones. Today predicates and date grouping use that zone, including daylight-saving boundaries, and never the server zone. `show_completed` includes completed matches in the selected view; the Completed selector always selects completed reminders.
  - Completion filtering applies before nesting. Manual ordering uses supported native ordering hints, with record id as a stable tie breaker. Missing due/creation/order values sort last; priority uses Apple's high (1), medium (5), low (9), none (0) semantics. Title uses case-insensitive, numeric-aware Unicode root collation (`Item 2` before `Item 10`) through the existing x/text dependency. The query has no viewer locale, so locale-specific Apple ordering remains an approximation.

**Reminders record additions:** Existing EventKit fields stay authoritative. Nullable store fields are unavailable rather than false/empty guesses; `store_status` is `complete|partial|unavailable|unsupported`, and `store_coverage` maps field names to `{status, reason?}`.

- `reminders.reminder`: `store_identifier`, `flagged`, `parent_id` (reminder record reference), `section_id`, `tags[]`, `manual_order`, `created_at`, `early_reminders[]` (signed seconds relative to due date), `assignment{assignee, originator, assigned_at}`, `store_alarms[]`, `store_recurrence[]`, `attachment_ids[]` (attachment record references). Store identity is linked by exact equality to EventKit `calendarItemIdentifier`, proven on a tagged fixture; external identifiers are aliases scoped by list, never an unverified fallback.
- `reminders.list`: `store_identifier`, `badge_emblem`, `is_group`, `parent_id` (list record reference), `display_order`, `pinned`, `pinned_at`, `sorting_style`, `categorize_groceries`, `grocery_locale`, `sharing_status`, `shared_owner_name`, `smart_list_type`, `smart_filter`, `sections[]`. Store color retains native representation separately as `store_color`; EventKit `title`, `color`, `account`, and `writable` remain available.
- `reminders.attachment`: owning `reminder_id`, `filename`, `uti`, `size`, `width`, `height`, `digest`, `derivatives`, and `media` through the existing blob/variant pipeline. Native paths never leave the Helper.

`assignment.assignee` is `{id, is_me: bool|null}`. The Helper verifies `is_me` from source identity; an unknown identity remains null. A known absence of an assignment is distinct from an unavailable assignment field.

Derived indexes update transactionally with apply and rebuild on replica snapshot installation. Reads apply visibility before counts, nesting, tags and smart predicates. No store-only field requires a second EventKit fetch. After an EventKit capture completes, enrich through a long-lived `SQLITE_OPEN_READONLY` connection and a short read transaction, including committed WAL. Confine the connection and reads to a dedicated Swift actor; capture completion awaits it off the Helper main actor. Cache prepared statements and typed rows; retain blobs as `Data` until their decoder is needed. Read Core Data persistent history (`ATRANSACTION`/`ACHANGE`) after the last valid token and reload only changed entities. Invalid or truncated history and schema changes require a full read; `Z_OPT` deltas are the fallback when history is unavailable, re-reading only tables that lack `Z_OPT`. Both history anchors must still exist; pruning past the token forces a full read. The independent consistent-copy reader remains a proof oracle, outside the capture path. No additional watcher in this unit.

Budgets: the shared REST/MCP read-model handler for 1,000-item `reminders.items` stays <10 ms p95 in the gateway. Operations marked `ReadModel=true` in their descriptor receive the same frontdoor timing without operation-name checks. Measure that computation server-side, before response framing: REST reports `Server-Timing: readmodel;dur=...`; MCP reports `_meta["bridge/read_model_ms"]`. Client round-trip timings are descriptive. MCP carries these read-model results in `structuredContent` plus the same serialized JSON in a text block for compatibility; the output is encoded once and its bytes reused for both representations. Store enrichment budgets are in §23.15. REM-STORE-READ runs at sim and real with tagged fixtures and cleanup. AppleTruth store checks are `limited` with reason `AppleTruth lacks Full Disk Access to the Reminders Core Data store`; EventKit truth remains available. UI-dependent fixtures wait for an unlocked session and carry exact deferred commands.

### 23.5 Remote

**Commands:**
- `bridge remote on [--name <host>] [--via tailscale-app|embedded]` asks the running Runtime to enable Remote. Embedded tsnet is the default; `tailscale-app` (or the compatible `system` spelling) explicitly selects the system app. A system node missing HTTPS/Funnel reports the enablement URL instead of changing unrelated configuration. `--via` overrides selection; embedded hostname defaults to `bridge`.
- System mode uses the LocalAPI client and ETag/If-Match read-modify-write, preserving existing handlers and unknown config fields. It allocates the first unoccupied allowed port among 443, 8443, 10000, including foreground/service reservations. The mapping proxies only to a dedicated loopback listener, separate from the local API, with a private 256-bit capability path known to that mapping. Direct loopback requests without it are refused. An ownership record is persisted before installation; restart removes a stale owned mapping before reinstalling. Off removes only the unchanged owned mapping, preserving concurrent unrelated edits.
- Embedded mode stores state in `<home>/remote/`, mode 0700, prints the Tailscale login URL and waits for authorization. Missing HTTPS or Funnel prints and opens its QueryFeature enablement URL and rechecks every 5 s until enabled. Login links are printed and opened by the waiting CLI. It then uses `ListenFunnel("tcp", ":443")`.
- The enabled state and selected backend persist across Runtime restarts. The public URL is stable and includes a nondefault port in system mode. Before printing the public URL and MCP URL, Remote probes public DNS and HTTPS through the public Funnel relay: OAuth discovery 200, MCP 401 with WWW-Authenticate, app 200 and a 60 s synthetic SSE stream. See [remote flow](spec/remote.md).
- `bridge remote off` closes public serving, including upgraded replica feed connections, within 1 s. `bridge remote status` prints state, backend, URL, pending link, TLS termination (on this Mac), certificate expiry and the last error; `--snippets` prints connection examples. Commands communicate with Runtime through private atomic state files under `<home>/remote/`; Runtime must be running for on/off to complete.
- Allowed on replicas, off by default. Tailnet devices reach the same hostname without Funnel.

**Public routing:**

| Path | Handler | Auth |
|---|---|---|
| `/`, app routes, `/assets/*`, `/web/*`, `/oauth/*`, `/.well-known/oauth-*` | Web | device session / OAuth |
| `/v1/*`, `/mcp` | Frontdoor (public-host mode) | `Authorization: Bearer` with remote-allowed principals: **agent** tokens (§23.6), or explicitly opted-in `bridge token create --remote` tokens during W1; any `Origin` header rejects; local machine tokens are refused remotely |
| `/v1/feed` | Replication on gateways; 404 on replicas | `relay/1` paired device-key authentication (§12); any `Origin` header rejects |

TLS ends on the Mac: inside `bridge` with tsnet in embedded mode, or in system tailscaled in system mode, using the ts.net certificate. Funnel relays cannot read traffic. Remote pins the public Host before dispatch. System mode recognizes HTTPS and forwarded source only through its dedicated capability-protected mapping; forwarded Host must exactly match the configured public host, forwarded Proto must be HTTPS, and forwarded source must be one valid IP. The shared Web handler sees the pinned public Host, HTTPS, and original client IP. Embedded mode extracts the original source from the TLS connection’s `ipn.FunnelConn.Src`. Local API requests never inherit proxy trust, and ignore forwarded headers. Bandwidth limits are Tailscale's (unpublished), so media variants (§23.7) keep transfers small.

### 23.6 Agent access

**Agent tokens:**
- **Principal:** `agent:<id>`, created in the web UI (or `bridge agent create --name --level --scope`) and shown once.
- **Scopes:** the source grants of §14.
- **Levels:**
  - `read`: read operations only.
  - `approve`: every command is admitted as `awaiting_approval`, a new disposition before `admitted`.
    - Device sessions receive an `approval` event, and the web UI lists the command with redacted arguments plus the full text for the owner.
    - Approving resumes the normal lifecycle. The expiry clock starts at approval, with a 24-hour approval window.
    - Rejecting finalizes as `rejected_by_owner`.
    - `wait_ms` returns the current status.
  - `full`: commands proceed directly, with the same Helper gates as interactive commands.
- **Defaults:** new tokens start at `approve`. Lab and test principals keep today's gates.
- **Activity log:** commands appear individually (redacted per §15), reads aggregated per minute per operation. Retention is 90 days.
- **W1 routing:** agent commands and owner approvals execute on the gateway only. Replicas serve local scoped reads and forward commands with signed identity, grants and level. Gateway approvals for replica agents use a fresh signed principal recheck (§12); the 24-hour approval window and TTL starting at approval remain the same. Owner approval live events recognize both local and replica-delegated `agent:` IDs.
- **Owner routes:** `GET /web/v1/approvals` returns pending commands with hashed argument summaries and owner-only full arguments; `POST /web/v1/approvals/{command_id}` accepts `approve` or `reject`. Device sessions alone may call these routes. The `web.1` `approval` event carries command IDs for refetch.

**MCP OAuth 2.1 (connector UIs such as claude.ai and ChatGPT):**
- Endpoints: `/.well-known/oauth-protected-resource`, `/.well-known/oauth-authorization-server`, `/oauth/register` (dynamic client registration), `/oauth/authorize` (a consent page in the web app that requires a device session, where the owner picks the level and scopes), and `/oauth/token`.
- PKCE S256 is mandatory.
- Access tokens last 1 hour; refresh tokens rotate, last 90 days, and are revocable from the Agents list.
- The resulting access token is an `agent:` principal.

**Connection snippets** generated per token:
- **Grok:** xAI SDK `mcp(server_url, authorization)`.
- **Claude Code:** `claude mcp add --transport http … --header`.
- **Codex:** `config.toml`.
- **ChatGPT and claude.ai:** connector URL with OAuth.
- **curl.**

### 23.7 Media derivatives

The Helper creates variants for present attachment blobs, as new blobs referenced from the attachment record:

| Input | Variants | Metadata (`media`) |
|---|---|---|
| Image (HEIC, JPEG, PNG, WebP, AVIF, TIFF) | `thumb` ≤ 400 px, `display` ≤ 1600 px; JPEG q0.8/0.85, or PNG when there's alpha; orientation applied | `width`, `height`, `thumbhash` |
| GIF | `thumb` (first frame); the original plays | same |
| Video (MOV, MP4, 3GP) | `video` H.264/AAC MP4 ≤ 1080p with faststart; `poster` JPEG from the source frame, 360 px on the short side, at most 1.5 MP | `width`, `height`, `duration_ms`, `thumbhash` |
| Live Photo (still + paired MOV) | image variants + `live` MP4; `poster` from the oriented key still, same size rule | + `live: true` |
| Audio (CAF, AMR, M4A, MP3) | `audio` AAC M4A | `duration_ms`, `waveform` (64 peaks, 0–255) |
| Sticker (HEIC/PNG with alpha) | `display` PNG | `width`, `height` |

- **Encoders:** ImageIO and AVFoundation, which use the hardware encoders.
- **Order:**
  - New attachments come first: images within 2 s, video when its export completes.
  - Backfill runs newest-first, one job at a time at utility QoS, and pauses below 20% battery or under serious thermal state.
  - Backfill yields to foreground capture at stage checkpoints instead of cancelling. Completed stages are reused by digest. A job whose source changes (device, inode, size or modification time; extended attributes don't count) restarts from a fresh digest.
- **Upgrades:** completed entries upgrade only their poster while the source is local; offline sources keep the cached poster. The cache format is unchanged.
- **Sync:** variants count toward blob caps and sync like other blobs. Replicas fetch variants before originals.
- **Fallback:** a request by the original digest serves a browser-safe original when a variant is missing, or `variant_pending` otherwise.
- **Exact variants:** clients request the selected derivative's digest. A pending derivative returns 503 `variant_pending` with `Retry-After`; an unavailable one returns 404 `variant_unavailable`; both are `no-store`. `immutable` caching applies only when the body's digest equals the URL digest.

### 23.8 Frontend

- **Stack:** `web/` with pnpm, Vite, SolidJS and TypeScript (strict). No UI libraries.
- **Source layout:**
  - `src/app`: shell, router, layouts, theme
  - `src/design`: tokens.css from `design/web/messages.md` §16, materials, type, SVG icon sprite
  - `src/motion`: springs, gestures, shared-element transitions
  - `src/virtual`: virtual list with variable heights and anchor preservation
  - `src/data`: `web/1` client, store, IndexedDB cache, optimistic commands
  - `src/auth`, `src/settings` (devices, passkeys, agents, approvals, activity)
  - `src/<app>` per app
- **Budgets** (Playwright gates):
  - shell JS ≤ 60 KiB gzip, measured over the `index.html` static-import closure; route chunks loaded lazily
  - each app-route lazy JS closure, including its lazy descendants and excluding shared shell chunks, ≤ 60 KiB gzip
  - record actual service-worker precache gzip bytes; the full cacheable distribution, including lazy assets and fonts, ≤ 600 KiB gzip
  - CLS 0
  - no long task > 50 ms during scroll or navigation
  - interaction-to-next-paint ≤ 8 ms p95 on the Air
  - warm load to cached content < 1 s
- **Lint:** `make lint` gains a zero-comments check for `.ts`/`.tsx` (the TypeScript tokenizer, so strings, templates and regexes are respected) and `tsc --noEmit`.
- **Build:**
  - `make web` runs `pnpm install --frozen-lockfile && pnpm build:release` into `web/dist/`, and `make build` depends on it. The lab uses `pnpm build` with browser fixtures.
  - `web/dist/*` is gitignored except `.gitkeep`, so `go:embed all:dist` always compiles.
- **PWA:**
  - Manifest with our own icons.
  - The service worker precaches the shell and caches blob thumbs in Cache Storage (200 MB LRU).
  - Public sessions get no caching.

### 23.9 Demo

`bridge-lab demo [--listen 127.0.0.1:0]` starts a sim World gateway seeded by `lab/demo`:
- **Dataset:** deterministic, synthetic and personal-data-free. About 40 chats (direct and group; iMessage, SMS and RCS) and about 5k messages, covering every item type the sim represents, with generated images, PDFs and vCards.
- **Output:** it prints a URL with an auto-login **demo session**. That session exists only in lab builds and is impossible in the product binary.
- **Uses:** contributors, README media and the `web` test level.

### 23.10 Test level `web` and scenarios

- **Level:** `web` is Playwright through `bridge-lab`. A scenario YAML with `level: web` runs `pnpm exec playwright test -g <ID>` against a fresh demo world.
- **Matrix:** WebKit, Chromium and Firefox × light and dark × phone (390×844) and desktop (1440×900), as each scenario declares. Passkey flows use Chromium's virtual authenticator.
- **Real-level web scenarios** drive the same UI against the real gateway on the Air.

W1 scenarios:

| ID | Level | Proves |
|---|---|---|
| WEB-SERVE-EMBED | sim, web | binary serves the app; immutable asset caching; route fallback; "not built" page |
| WEB-AUTH-ENROLL | web | enroll token → passkey → trusted session; token single-use and expiring |
| WEB-AUTH-LOGIN | web | discoverable sign-in; trusted vs public (public leaves no IndexedDB/Cache Storage) |
| WEB-AUTH-REVOKE | web | revoke closes the other socket ≤ 1 s and wipes its cache; `bridge web lock` |
| WEB-AUTH-ORIGIN | sim | foreign or missing Origin on state change or WS upgrade rejects; CSP and headers present |
| WEB-AUTH-RATELIMIT | sim | sign-in and enroll limits |
| WEB-RM-CHATS | sim | `messages.chats` equals Truth-derived summaries via REST and MCP; budget |
| WEB-RM-THREAD | sim | thread pages stable, no gaps or duplicates under concurrent inserts; reactions folded; budget |
| WEB-LIVE-DELTA | web | sim inbound message reaches the browser view ≤ 500 ms p95 (browser-observed) |
| WEB-LIVE-RESUME | web | socket drop and reconnect converge without gaps |
| WEB-MEDIA-VARIANTS | real | HEIC, MOV and CAF attachments on the Air get variants with correct dimensions and duration, and play in WebKit, Chromium and Firefox |
| WEB-REMOTE-FUNNEL | real | `bridge remote on` → public URL reachable from Ubuntu with a valid certificate; agent REST/MCP works; local tokens refused; `off` closes ≤ 1 s |
| WEB-AGENT-LEVELS | sim | read rejects commands; approve → `awaiting_approval` → approve executes / reject finalizes; full executes |
| WEB-AGENT-OAUTH | sim, web | dynamic registration + PKCE + consent with a device session → MCP call succeeds; refresh rotates; revoke works |
| WEB-PERF-SHELL | web | bundle, CLS and warm-load budgets |

### 23.11 W1 units

| Unit | Contents | Depends on |
|---|---|---|
| W1-RM | §23.4 read models + descriptors + WEB-RM-* | — |
| W1-WEB | §23.2 + §23.3 server side (Web module, sessions, passkeys, live views, blob variants route, interactive principal marker) + WEB-AUTH-* (server parts) | W1-RM for live views of chats and threads (stub until merged) |
| W1-UI | §23.8 scaffold: tokens, motion engine, virtual list, layouts, router, `web/1` client, auth and settings screens, PWA, lint, `make web`, embed, Playwright harness and `web` level | protocol in §23.3 (parallel with W1-WEB) |
| W1-MEDIA | §23.7 + WEB-MEDIA-VARIANTS | — |
| W1-REMOTE | §23.5 + WEB-REMOTE-FUNNEL | W1-WEB |
| W1-AGENT | §23.6 + WEB-AGENT-* | W1-WEB, W1-REMOTE |
| W1-DEMO | §23.9 | W1-RM, W1-UI |
| W1-RELAY | replicas over Remote (`/v1/feed`, replica device-key auth instead of TLS client certs) | W1-REMOTE; lowest priority |

### 23.12 W2: Messages, reading at full fidelity

**Scope:** every **W2** row of `design/web/messages.md`. W2 follows the draft spec. Rows Johan changes after his skim are applied when he gives them; this doesn't block W2.

**Reader decoding (Swift Helper → records), added to §6.2.** Decoders are pure and fixture-tested (§19), with fixtures scrubbed to synthetic values.

`messages.message` gains:
- `balloon{bundle_id, kind, app_name?, link?, app?}`:
  - `kind` is one of `link|app|digital_touch|handwriting|poll|check_in|apple_cash|find_my|chatbot|unknown`.
  - `link`: `{url, title?, summary?, site_name?, image_attachment_id?, icon_attachment_id?, video_url?}`, from `payload_data` (keyed archive) plus the plugin payload attachments.
  - `app`: `{caption?, subcaption?, trailing_caption?, image_attachment_id?, url?}`, from the MSMessage template layout when present.
- `audio{is_audio_message, played_at?, expirable, kept, transcription?}`.
- `schedule{state, scheduled_for}` (Send Later).
- `subject`.
- `delivery{quietly, notified_anyway}`.
- `is_spam`, `has_unseen_mention`, `via_satellite`, `is_sos`.
- `formatting[]`: `{utf16_location, utf16_length, styles[bold|italic|underline|strikethrough], effect?[big|small|shake|nod|explode|ripple|bloom|jitter]}`, from attributedBody attributes.
- A `genmoji` kind in `parts[]`: `{attachment_id, description}`.

`messages.attachment` gains:
- `is_sticker`.
- `sticker{target_message_id, part_index, x, y, scale, rotation}`, from `sticker_user_info`.
- `hidden` (`hide_attachment`).
- `genmoji`.

`messages.chat` gains:
- `filter: known|unknown`, from `is_filtered` and contact linkage.
- `group_photo_attachment_id?`.
- `read_receipts: on|off|default`.
- `muted?` is the known Hide Alerts boolean, independent of an expiry. `muted_until?` and `pinned_index?` are emitted only when reliably captured. Unknown values stay null/absent with `coverage.missing`; a permanent mute never invents an expiry. A known `muted` boolean takes precedence over the legacy expiry projection.

Recently Deleted becomes `deleted_at` + `recoverable_until` on messages and chats, from the recoverable tables.

**Read models:**
- `messages.chats` filters `known|unknown|unread|deleted|spam` become real.
- `messages.collection` `links|locations` become real.
- `has_unseen_mention` is real.
- New read `messages.handles{ids[]≤500}` → `{handles: [{handle_id, address, service, contact_id?, display_name, given_name?, family_name?, initials, avatar{digest, variant}?}], cursor}`, for sender names and avatars in threads.

**Client architecture (`web/src/messages/`):**

| Path | Owns |
|---|---|
| `routes.ts` | `/messages`, `/messages/<chat>`, `/messages/<chat>/<message>`, `/messages/new`, `/messages/search?q` |
| `store/` | chats view store, thread window store (anchored window over `web.1` thread view, prefetching ±2 pages), handles cache (`messages.handles`, batched per frame), unread and badge |
| `format/` | dates and times (§2/§3 rules, locale-aware), names ("A, B & 2 others"), preview wording (§15), byte sizes, durations |
| `list/` | ConversationList (virtualized), ConversationRow, PinnedGrid, FiltersMenu, SearchField, Avatar (photo / monogram / group composite) |
| `transcript/` | Transcript (bottom-anchored virtual list), layout engine (runs, tails, headers, receipts as pure functions over ThreadItems), Bubble, TextContent (data detectors, mentions, formatting), Reactions, ReplyConnector, ThreadOverlay, GroupEventLine, Receipt, NewMessagesPill |
| `media/` | MediaBubble (single, stack, grid), VideoBubble, AudioBubble (waveform player), FileChip, ContactCard, LinkCard, StickerLayer, Genmoji, PlaceholderCard, MediaViewer (shared-element zoom, swipe, video controls, PiP, share and download) |
| `effects/` | one canvas layer with GPU-only animation for bubble effects (slam, loud, gentle, invisible ink), screen effects (echo, spotlight, balloons, confetti, love, lasers, fireworks, celebration) and text effects (big, small, shake, nod, explode, ripple, bloom, jitter). Each ≤ 2.5 s, skippable, replaced by a static label under Reduce Motion |
| `details/` | DetailsInspector (pane ≥1024, sheet below), member list, Collections tabs |
| `search/` | SearchResults (sections, filter tokens, highlighted matches, jump-to-message with pulse) |

**Rules:**
- **Chats cache:** trusted sessions atomically persist a versioned materialized view with its cursor, capped at 2,000 chats and 2 MiB serialized in display order. Hydration renders this snapshot before reconciling a fresh live view; transport pages do not independently hydrate or persist. IndexedDB upgrade removes legacy chat-page keys. Public sessions persist nothing, and session removal wipes the snapshot. WEB-MSG-CHATS-CACHE covers interrupted cross-page reorder, offline reload, upgrade and privacy.
- **Focused replies:** ThreadOverlay owns a bounded, paged `thread` subscription with `thread_originator_id`, so replies outside the main transcript window remain available without loading the whole chat. Its five 40-item pages cap the focused window at 200 items; the main transcript retains five 100-item pages. The focused reader never feeds pending-command projection or main-window observation. Concurrent transcript instances keep actual message IDs for actions and use separate DOM IDs.
- **Layout engine:** the transcript layout is a pure function `(items, handles, prefs, width) → rows`, with code-level tests allowed as layout math. Rows have deterministic height estimates from measured text plus known media dimensions (`media.width/height`). The DOM is authoritative for rendered text height; ResizeObserver corrects virtual positions with scroll-anchor compensation; residual estimate differences may require one frame of correction without clipping text. Media reservations remain fixed.
- **Text measurement:** done in one batched pass per frame using the exact computed text-node font family, size, weight, letter spacing, feature settings and variation settings. Canvas estimates reserve `ceil(width) + max(1 px, 0.5% of width)` before applying the bubble width cap. Nondefault font features or variations use batched DOM text measurements. Font-set readiness and matching FontFace loaded promises invalidate measurements, with an intrinsic font probe detecting later changes without depending on loadingdone events; intrinsic text height and tail silhouettes follow the observed DOM bounds.
- **Viewer source:** the media viewer pages the whole chat through `messages.collection{chat_id, kind: media}`, independently of the transcript window. Owning `message_id` supports Show in Chat; `sent_at` preserves interleaved photo/video order.
- **Media sizes:** media bubbles size from `media` before any byte loads. A ThumbHash placeholder fades to the image on decode, which happens off the main thread with `decode()`. Phone images use at most 70% of transcript width and 60% of viewport height; opaque portraits exceeding the height cap crop at the center. Alpha images and stickers preserve their aspect within those bounds and float without fill, radius or clipping. Desktop media width is at most 480 px.

**Performance acceptance (replaces rAF-interval proxies on 60 Hz/headless hosts):**
- **Main-thread work per frame** (script + style + layout, measured in Chromium tracing and with in-page instrumentation in WebKit and Firefox) ≤ 4 ms p95 and ≤ 8 ms max during:
  - a 100k-message transcript fling
  - list scroll
  - push/pop transitions
- **Long tasks:** none during those journeys. Chromium is the gate, because the Long Task API isn't available elsewhere.
- **Frame intervals** are still recorded but are only gates when the host display supports 120 Hz.
- **Input responsiveness** ≤ 8 ms p95 for send, react, open chat and open media. This is our own main-thread cost for the input: Event Timing input delay plus processing (`processingEnd − startTime`), plus the script, style and layout work of the frame that renders the result.
  - Click→next-frame (rAF) is recorded as descriptive data and becomes a gate only on a 120 Hz host. On 60 Hz or headless hosts the frame cadence alone is about 16.7 ms, so it measures display alignment, not our work.
  - Decided 2026-10-21 during W2-LIST acceptance; the measured list was 0.3–0.6 ms processing and frame work p95 1.93 ms.
- **WebKit:** WebKit jank must be explained and fixed. Show where the time goes (script, layout or paint) before claiming it's a headless artifact.

**W2 units:**

| Unit | Contents | Depends on |
|---|---|---|
| W2-READER | Decoding and records above, AppleTruth checks, SimApplePeer + demo dataset support for every new field, read-model filters and collections, `messages.handles` | W1-DEMO merged (for demo support) |
| W2-LIST | `list/`, `format/`, `store/` (chats, handles), search field, keyboard navigation, live reordering with springs | W1-DEMO |
| W2-TRANSCRIPT | `transcript/` core (text, runs, tails, headers, receipts, edits, unsends, reactions, replies and ThreadOverlay, group events, paging, anchors, new pill), the performance acceptance above, and the WebKit jank fix | W1-DEMO |
| W2-MEDIA | `media/` bubbles and MediaViewer, link and app cards, stickers, Genmoji, placeholders, and `effects/` | W2-TRANSCRIPT layout engine, W2-READER fields |
| W2-DETAILS | `details/`, `search/` results | W2-LIST |

**W2 scenarios:**
- Level `web` on the demo world, all 12 projects unless noted:
  - WEB-MSG-LIST, WEB-MSG-AVATAR-READY (delayed handle names, neutral avatars and stable monogram geometry), WEB-MSG-LOADING (cached first paint and delayed skeletons), WEB-MSG-REORDER (live ordering and unread springs), WEB-MSG-FILTERS, WEB-MSG-SEARCH, WEB-MSG-THREAD-CORE, WEB-MSG-REACTIONS, WEB-MSG-REPLIES, WEB-MSG-EDIT-UNSEND-VIEW, WEB-MSG-GROUP-EVENTS, WEB-MSG-MEDIA, WEB-MSG-VIEWER, WEB-MSG-LINKS, WEB-MSG-STICKERS-GENMOJI, WEB-MSG-EFFECTS, WEB-MSG-PLACEHOLDERS, WEB-MSG-DETAILS, WEB-MSG-KEYBOARD, WEB-MSG-A11Y (axe-free structural checks plus labels), WEB-MSG-PERF.
  - WEB-PERF-LIST uses one Chromium/light/desktop project with `web.huge: true` for list scroll, filter switching, search typing and live reorder. It runs in the exclusive lane with frame work p95 ≤ 4 ms / max ≤ 8 ms and own-input cost p95 ≤ 8 ms. WEB-MSG-PERF is transcript-only and its canonical manifest is owned by W2-TRANSCRIPT.
  - WEB-MSG-SIDEBAR separately verifies desktop sidebar resize bounds, RTL keyboard handling and trusted-only width persistence. Its separate World avoids unrelated fixture authentication bursts.
  - WEB-MSG-SEARCH-LIST covers inline conversation matches, search-field shortcuts and the full-results route hook; WEB-MSG-SEARCH covers the full results page. WEB-PERF-SEARCH checks the unchanged 50 ms response-to-DOM budget for bounded results at the Messages milestone.
  - WEB-MSG-KEYBOARD-LIST covers list selection, chat cycling and pinned shortcuts; WEB-MSG-KEYBOARD covers transcript shortcuts.
  - WEB-MSG-LIVE: a sim inbound message appears with the receive animation in under 500 ms p95.
- Level `real` (the real gateway on the Air, read-only):
  - WEB-MSG-REAL-READ: the real chat list and threads render without errors.
  - Every decoded field kind present in real data renders. Evidence is counts only.
- Reader level `sim` + `real`:
  - MSG-DECODE-BALLOON, MSG-DECODE-FORMATTING, MSG-READ-RICH-FIELDS (AppleTruth agrees on counts per field kind).

### 23.13 W3: Messages, writing

**Interactive gate (Helper).**
- **Where interactive comes from:** a command arriving over ApplePeer with `origin: interactive`. Only the gateway sets this for `device:` principals (§23.2) and policy-authorized `agent:` principals at level `full` (§23.6); the IPC socket is owner-only. The Helper rejects unknown origins, inconsistent action origins, and interactive origins from other principal kinds at the IPC boundary. Dispatch must retain the prepared origin.
- **What it relaxes:** interactive commands skip the lab text tag (`[ab-test <id>]`) and the own-handle route restriction.
- **What still applies:** the operation's capability proof for this OS build, expiry, the dispatch guard, conservative correlation, and the no-redispatch rules (§11.2).
- **Other principals:** lab, test and `agent:` principals keep today's gates. `agent:` with level `full` gets interactive-equivalent routing only after owner approval or at level `full`, per §23.6.

**Self-test (`bridge selftest messages`, run by `bridge setup`).** It proves each Messages operation on this Mac's OS build and records capability evidence (ADR 0008).
- **Own handles:** discovered from the Messages account (`account_login` / `destination_caller_id`) and the Contacts "me" card.
- **Run:** in a tagged self chat it does send text, send a file, reply, react (classic and emoji, add and remove), edit, unsend, mark read/unread, and delete message. Afterwards it cleans up everything it created.
- **Result:** each passing operation becomes `proven` for the build; a failure leaves that operation hidden in the UI with the reason.
- **Rerun:** automatic after a macOS update changes the build. The Helper caches one report per running build and does not repeat an interrupted run; `bridge selftest messages --force` starts a new, guarded run. `bridge setup` invokes the same self-test. `--no-ui` proves the AppleScript steps and records the remaining operations and cleanup as deferred. Private run manifests and the dispatch guard retain unfinished cleanup obligations; failures remain unproven with per-operation reasons.

**Proofs to finish at the real level** (sim-only today): `messages.mark_read`, `messages.mark_unread`, `messages.delete_message` and `messages.delete_chat` (tagged self chat only), plus resend of a failed message (`messages.send` with `resend_of`). Resend names a visible failed outgoing message in the same `chat_id` and always uses a fresh command ID. Operations validates that reference, and the Helper rechecks native failure before preparation and dispatch. Reusing the resend command ID returns its existing outcome. Indeterminate effects are never automatically resent. The real proof requires an explicitly supplied registered failed self-message fixture; it never induces an unsafe native failure.

**Upload path:**
- `POST /web/v1/upload` streams to a staged blob on the serving instance.
- **On a replica,** the serving instance verifies its staged bytes and supplies their mandatory digest to signed feed upload controls. Bytes use the binary bulk lane in 1 MiB chunks (§12), with the digest verified by the gateway before readiness and by the replica against the ready response, before the referencing command is admitted.
- **Limits:** 100 MiB per file, 10 files per message.
- Gateway-local raw uploads retain their local staging path. A completed response carries `upload_id`, `offset`, `size`, `state: ready` and `expires_at`.
- Optional resumable staging starts with an empty POST and `Upload-Length`; `Upload-Digest` optionally checks the browser bytes against a supplied lowercase SHA-256 digest; replica forwarding always supplies and verifies a mandatory digest on both transfer endpoints. Append raw bytes with `Upload-ID` and `Upload-Offset`. `GET /web/v1/upload/{id}` returns progress; `DELETE` abandons it. IDs are principal-owned. Accepted writes extend the lease to ten minutes; status reads and commit do not extend it. Interrupted writes retain acknowledged offsets. Completion verifies the declared size and digest before readiness. On a replica the final append returns only after gateway verification and may take as long as forwarding; a client that loses that response re-reads status and sends a zero-byte append to resume.
- Expired or abandoned leases release temporary files, upload aliases and pins. Before admission (including awaiting owner approval), the gateway verifies ready upload IDs and persists their blob references with the command. Durable command protection survives upload lease expiry and owner approval; an upload ID alone never grants another principal access.

**Client (`web/src/messages/compose/`, `actions/`):**
- **Composer:** text, Enter/Shift+Enter, autogrow to 6 lines, paste, drag and drop, file picker, phone camera capture, audio recording sent as a file (an audio message in W4), location sent as a maps link, our own desktop emoji picker (⌃⌘Space), and per-chat drafts in local storage. Public sessions keep drafts in memory only.
- **Sending state:** pending → sent → delivered/read per part. `command.parts` carries `{index, kind: text|attachment, upload_id?, record_id?, disposition}` (§11.2). In a partial failure, only failed parts show Not Delivered; Try Again sends only the selected failed part with a fresh command and its own `resend_of`, leaving successful parts intact. Unverified failures cannot enable Try Again, and indeterminate parts use §3.1's Check/notice policy.
- **Message actions** (long-press, right-click, ⌘T/⌘R/⌘E/⌘Z):
  - Tapback picker (6 classic, emoji search, remove).
  - Reply mode.
  - Inline edit (15 min, 5 edits).
  - Undo Send (2 min; particle dissolve).
  - Delete (confirm).
  - Copy, Forward (pre-filled new message), Save attachment.
- **Chat actions:** mark read/unread (swipe and menu), delete chat (confirm, full-swipe).
- **New message:** recipient tokens with `contacts.lookup` autocomplete, service hint from an exact-participant `messages.chats` lookup, an existing chat loads under the To field, and send. History lookup never blocks a single-recipient Send; a failure only shows "Couldn't load earlier messages". Sending to a handle lets native Messages select or create the chat only after send succeeds. Two or more tokens show "New group messages aren't available yet" beside disabled Send, while an exact existing group can still load read-only history.
- **Optimistic UI:** each pending part has a distinct presentation key and swaps only by its own exact `record_id` at a compatible captured cursor reaching `commit_seq`. Actual captured IDs and versions remain unchanged. No text, filename or time matching is permitted. Multi-file app sending stays disabled until per-part mapping and cardinality proof land. It never duplicates and never flickers.
- **Gesture guard:** every command is submitted from input-event handlers only (§23.2).

**Scenarios** (level `web` on the demo world, plus `real` on the Air with Johan's own handles only; never text anyone else):

| ID | Levels | Proves |
|---|---|---|
| MSG-INTERACTIVE-GATE | sim, real | interactive skips the tag and own-handle rules; lab and agent principals don't; expiry and no-redispatch unchanged. At real level, an untagged interactive send to own handle, then cleanup |
| MSG-DELETE-VISIBILITY | sim, real | sim checks retained recoverable message/chat records and restoration; read-only real proof compares a previously cleaned test message with independent native Truth |
| MSG-MEDIA-CLEANUP | sim | native manifest authorization and unambiguous attachment-only target selection; no effects on unproven data |
| MSG-SELFTEST | real | the self-test proves each operation and cleans up completely |
| MSG-READSTATE-REAL, MSG-DELETE-REAL | real | the conversation proofs listed above, using an isolated tagged self chat |
| MSG-RESEND | sim, real | explicit fresh-command resend of a failed message; real proof requires a registered failed self fixture |
| MSG-COMMAND-PARTS | sim | text plus two files captures distinct part identities, including concrete-row and no-effect failures plus same-aggregate partial evidence progress; REST/MCP, live command status and owned command-record parity. Real proof waits for unlock with tagged self-handle cleanup |
| WEB-MSG-SEND | web, real | text send: tap to local bubble ≤ 1 frame; ack; delivered; no duplicate |
| WEB-MSG-ATTACH-SEND | web, real | photo, video, PDF and audio-file upload and send, including via a replica-served UI |
| WEB-MSG-REACT, WEB-MSG-REPLY, WEB-MSG-EDIT, WEB-MSG-UNSEND | web, real | each action end to end, with the UI's time limits |
| WEB-MSG-CHAT-ACTIONS | web | read/unread and delete chat via swipe and menu |
| WEB-MSG-NEW | web | new message with autocomplete and service hint |
| WEB-MSG-FAIL-RESEND | web | failure shows "Not Delivered" and Try Again resends |
| WEB-MSG-OPTIMISTIC | web | reconnect mid-send converges with no duplicates |

**Units:**

| Unit | Contents |
|---|---|
| W3-HELPER | interactive gate in Swift, self-test, real proofs, resend |
| W3-UPLOAD | replica upload forwarding, upload limits |
| W3-COMPOSE | composer, actions, chat actions, new message, optimistic UI, emoji picker |

### 23.14 W4: Messages parity

**Approach.** Each item gets:
- a Helper implementation through Accessibility or AppleScript (ADR 0009)
- capability proof per OS build (an added self-test step where it can be done on self)
- a sim scenario and a real scenario on self, or on the Ben RCS contact where a group is required

Items needing a second iMessage identity are `limited` at the real level (DECISIONS 2026-10-16).

| Item | Read | Write |
|---|---|---|
| Pins (order, pin/unpin, reorder) | §23.12 if readable; else via Accessibility snapshot of the sidebar on change | Accessibility |
| Hide Alerts | as above | Accessibility (details pane toggle) |
| Read receipts per chat | §23.12 | Accessibility |
| Send Later (create, edit time, send now, delete) | §23.12 | Accessibility (+ menu → Send Later) |
| Send with effect | — | Accessibility (effect picker) |
| Formatting and text effects | §23.12 | Accessibility (format menu over a selection) |
| Mentions | §23.12 | type `@name` and confirm the suggestion via Accessibility |
| Audio message | §23.12 | send the recorded file as an audio message (Accessibility or the file-flag route; investigate) |
| Keep expiring audio | §23.12 | Accessibility |
| Recently Deleted (recover, delete now) | §23.12 | Accessibility |
| Groups (create, rename, photo, add, remove, leave) | existing | Accessibility; real proofs RCS via Ben; iMessage-group parts `limited` |
| Typing indicators | Accessibility tree of the open chat (investigate) | not feasible without driving the Mac's UI per keystroke; hidden unless reading proves reliable |
| Link preview on send | sender-side preview generation | investigate whether Messages generates it for AppleScript sends; otherwise hidden |

**Helper A command contract.** `messages.pin({id,pinned_index})` uses -1 to unpin and 0–8 to insert/reorder; the pin set stays contiguous and has at most nine items. `messages.mute({id,muted})` records a known boolean without inventing a mute expiry. `messages.set_read_receipts({id,read_receipts})` accepts on/off/default. `messages.recover_chat({id})` and `messages.delete_chat_permanently({id})` require a recoverable conversation. All require `expected_version`; recovery and purge affect only that conversation's recoverable members. Simulation proves these transitions; it does not establish native support.

The signed Helper's `selftest messages-parity --report <private-report>` uses an unused own handle and registers cleanup before sending. It never falls back to an existing conversation. Its per-OS-build ledger publishes exact failure reasons and leaves unproven operations out of capabilities. The 25G241 attempt returned an indeterminate isolated-fixture send; no native parity variant was enabled. Tagged cleanup found zero active messages; the late-effect reconciliation registration is retained. Mute capture may use only a property independently identified by an AX/native-properties round trip, and only for the service used in that proof; it never guesses a key. Pin order, receipt mutation and exact Recently Deleted AX targets remain unproven and hidden.

**UI:** the W4 rows of `design/web/messages.md` (pinned grid drag reorder, mute bell, Send Later bubble and sheet, long-press send effect picker with live preview, formatting toolbar, mention suggestions, audio message recorder with waveform, Recently Deleted actions, group management in details).

**Units:** W4-HELPER-A (pins, mute, read receipts, Recently Deleted), W4-HELPER-B (Send Later, effects, formatting, mentions, audio message, Keep), W4-GROUPS, W4-UI, W4-TYPING (investigation, may close as hidden).

### 23.15 W5–W8: Reminders, Calendar, Contacts, Notes

Each app follows its spec (`design/web/reminders.md`, `calendar.md`, `contacts.md`, `notes.md`) in three steps. Each step starts with failing scenarios at sim/web and ends at real on the Air, using tagged "Bridge Test" containers and cleanup:
- **R:** read at full fidelity, including store-backed fields beyond the public frameworks, with AppleTruth checks, read models and the UI.
- **W:** basic writes through the existing commands, with the UI and optimistic state.
- **P:** parity through AppleScript or Accessibility, proven per OS build or hidden.

| Milestone | App | Store reader (R) | Read models |
|---|---|---|---|
| W5 | Reminders | Reminders Core Data store (read-only WAL snapshot, FDA) linked to EventKit identifiers | `reminders.lists`, `reminders.items` |
| W6 | Calendar | Calendar store for travel time, attachments, geo and invitation status | `calendar.range` with overlap groups |
| W7 | Contacts | — (Contacts framework is complete except notes) | `contacts.list` |
| W8 | Notes | Extended NoteStore protobuf decoder (full blocks) | `notes.list` |

**W5 units (Reminders), runnable as worker capacity frees:**

| Unit | Contents |
|---|---|
| W5-STORE | Swift store reader per the `reminders.md` §7 mapping. Pure blob decoders (early-reminder deltas, section order and membership, smart-list filter) with scrubbed fixtures. AppleTruth counts. SimApplePeer + demo dataset fields. Read models `reminders.lists` and `reminders.items` (smart, tag and section aware). Scenarios REM-STORE-READ (sim, real). |
| W5-UI | Sidebar and home (smart tiles, lists with groups, tags), list content (rows, sections, subtasks, completed toggle, sort), details inspector (read), search, keyboard, motion. WEB-REM-READ and WEB-REM-DETAILS. |
| W5-WRITE | Inline create with client-side natural-language dates, edit, complete (signature motion), delete, move, create list; optimistic via `observed_record_ids`. WEB-REM-WRITE (web, real on "Bridge Test"). |
| W5-PARITY | Flag, subtasks, sections, tags, list icon/emoji/group, manual reorder, images, custom smart lists, through AppleScript or Accessibility, proven per OS build or hidden. WEB-REM-PARITY. |

The read inspector renders EventKit recurrence frequency, interval, selectors and end conditions, absolute/relative alerts, and location proximity, radius and coordinates in the viewer’s time zone. Recurrence is labeled as the next incomplete occurrence. Unknown schedule shapes hide the affected field and glyph rather than guessing. WEB-REM-DETAILS verifies synthetic values on desktop and phone. Schedule configuration is read-only; basic edits preserve these fields.

Verified manual ordering: `ZREMINDERIDSMERGEABLEORDERING_V2_JSON` is an unversioned JSON array of canonical UUID strings on macOS 26.7.1. The pure decoder accepts only that shape with unique identifiers and exposes zero-based `manual_order` positions. Invalid bytes, duplicate identifiers, object/version envelopes, and unverified formats yield null with unsupported coverage. A supported array that omits a reminder yields null with partial coverage. List-only order changes compare decoded positions and reproject only reminders whose positions or coverage changed; appending an identifier does not reproject unchanged positions. Unchanged ordering bytes reuse the decoded list ordering. The scrubbed fixture comes from a tagged three-reminder list created and deleted by the Helper. A second guarded five-reminder probe verified the same ordering format; neither absolute nor relative EventKit alarms populated due-date delta blobs; positive early-reminder, section and smart-filter fixtures remain deferred while the Air is locked, so those fields stay unsupported.

W5-STORE budgets on the Air: incremental enrichment after each EventKit capture <50 ms p95; 1- and 10-reminder changes <5 ms p95. Report 200-sample p50/p95 for 1, 10, 100 and 1,000 changed reminders. Full reads run at Helper startup or fallback (invalid history token, history truncation, schema change) and stay <100 ms p95 warm. Report cold reads separately without a gate, since they include page-cache warmup. A fresh forced full read must keep main-queue blocking below 16 ms.

Real live runs time the full `RemindersStoreEnrichment.refresh` await inside the Helper, report the first call separately and retain warm p50/p95/max plus per-capture timings. Compare real main/candidate runs as interleaved A/B/A/B with per-effect timestamps and load/swap; repeat a pair if either run starts above 7 GB of swap. Both candidates must pass the existing ≤1 s live budget; candidate p95 stays within the baseline-run spread plus 10 ms, and direct warm enrichment p95 stays below 50 ms.

Store refresh piggybacks on capture completion; it adds no EventKit fetch or watcher. Known gap: a store-only flag, section or tag change reaches Bridge only if EventKit also posts a change notification. Once the Helper has Reminders Apple Events authorization, the tagged real flag proof must measure flag-only gateway propagation without another edit. If no notification arrives, follow up with a WAL FSEvents hint through the existing bounded coalescer (§7), never one refetch per hint.

Measure the first Reminders checkpoint after Helper start before and after enrichment; if enrichment adds more than about one second, emit the EventKit checkpoint first and store fields in the next checkpoint.

**W6 units (Calendar):**

| Unit | Contents |
|---|---|
| W6-STORE | Store-backed fields per the `calendar.md` §4 mapping, AppleTruth counts, the read model `calendar.range` with precomputed overlap groups (≤ 10 ms p95 for a month), sim and demo dataset. CAL-STORE-READ. |
| W6-UI | Day, Week, Month, Year and List views, calendars sidebar, mini month, event inspector (read), search, now-line, period paging with momentum, keyboard. WEB-CAL-READ. |
| W6-WRITE | Create (drag, double-click, quick event with client-side natural language), move and resize with snapping, edit and delete with the This/Future prompt, alerts, time zone, new calendar. WEB-CAL-WRITE (web, real on the "Bridge Test" calendar). |
| W6-PARITY | Invitation responses, travel time and attachments through Calendar.app Accessibility where provable, otherwise hidden. WEB-CAL-PARITY. |

**W7 units (Contacts):**

| Unit | Contents |
|---|---|
| W7-READ | Read model `contacts.list{group?, sort}` with section keys and display/sort-order preferences, photo variants, "me" identification, linked conversations through `messages.handle.contact_id`. UI: lists sidebar, A–Z sections with an index scrubber, card with labeled fields and quick actions that open `tel:`/`facetime:`/`mailto:` or Messages new-message, vCard download. WEB-CON-READ. |
| W7-WRITE | Inline edit mode (add/remove fields, label pickers, photo set/remove), create, delete with confirm, optimistic via `observed_record_ids`. WEB-CON-WRITE (web, real on the "Bridge Test" organization). |
| W7-PARITY | Group membership writes through `CNSaveRequest`, and the contact poster if stored accessibly. Otherwise hidden. WEB-CON-PARITY. |

**W8 units (Notes):**

| Unit | Contents |
|---|---|
| W8-DECODE | Extend `Decoders.NotesProto` to full block structure: titles, headings, monostyled, lists, checklists with state, tables, links, quotes, highlights, collapsible sections, and attachment references with blobs and variants. Pinned and tags. Scrubbed fixtures and AppleTruth counts. Read model `notes.list{folder, sort}`. NOTE-DECODE-FULL (sim, real). |
| W8-UI | Folders sidebar (accounts, nested folders, Recently Deleted read-only), note list (pinned section, date groups, previews, thumbnails, gallery), read-only rich editor rendering, locked notes title-only, search, keyboard. WEB-NOTE-READ. |
| W8-WRITE | Plain-text editor for `editable: plain` notes with debounced autosave on `expected_version`, new note and folder, delete. WEB-NOTE-WRITE (web, real in the "Bridge Test" folder). |
| W8-PARITY | Investigate checklist toggling and simple rich edits through AppleScript HTML bodies on losslessly round-tripped notes; keep only what's provable. WEB-NOTE-PARITY. |

Notes rich records retain `plain_text`, escaped fallback `html`, structured `blocks`, attachment references and optional `pinned` / `tags`. The block/run contract and unknown-format behavior are specified in `design/web/notes.md` §5. Locked bodies are never decoded or staged. A serializable block tree is not itself a write qualification: rich editing stays hidden until its per-OS proof passes. Recently Deleted is captured read-only; a delete may be observed as a move there rather than a tombstone.

W8-PARITY is limited on macOS 26.7.1 (25G241): the tagged simple-rich HTML replacement did not preserve expected text/block structure, and public HTML checkbox import did not create native checklist identities. Neither rich edits nor checklist toggles are exposed. `NOTE-EDIT-PARITY` records the per-OS result with tagged-note cleanup; app acceptance remains a separate milestone gate.

The shell's app switcher enables each app when its R step passes. Scenario families: `WEB-REM-*`, `WEB-CAL-*`, `WEB-CON-*`, `WEB-NOTE-*` (web level, 12 projects), plus `REM-STORE-*`, `CAL-STORE-*` and `NOTE-DECODE-FULL` (sim and real).
