> Historical: written while the gateway and lab were Go. The Go code was deleted; the Rust gateway is `rust/crates/ab-runtime` and the lab runner is `rust/lab`. Go file and package names below are kept as the record of what was verified.

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

### 1.3 Safety rules for every test and agent (from the owner)

- iMessage tests text only the owner himself (his own number or iCloud email, discovered from his own contact card or Messages). Group tests may include the authorized group-test participant (handle in `~/.config/apple-bridge/test-contacts.env` on the test Mac, never committed). Never text anyone else.
- Test data is tagged and cleaned up: calendar **"Bridge Test"**, reminders list **"Bridge Test"**, contacts with organization **"Bridge Test"**, notes folder **"Bridge Test"**, message text containing **`[ab-test <id>]`**.
- Never modify or delete the owner’s data that tests didn't create. SIP stays on. No security setting changes.
- Never commit personal data (numbers, emails, message text, usage numbers, IPs, tokens). Fixtures are scrubbed with synthetic values that keep binary structure valid.

---

## 2. Glossary summary

The canonical glossary is `GLOSSARY.md` (domain language only). Key terms: **Source, Gateway, Replica, Helper, Bridge record, Collection, Observation, Capture, Source generation, Source checkpoint, Journal, Epoch, Subscription revision, Replication cursor, Snapshot, Command, Admission era, Ticket, Evidence, Disposition, Locally observed, Delivered, Indeterminate, Coverage, Freshness, Principal, Scope, Grant, Pairing, Delegated identity, Blob, Record grant, Reference set, Operation descriptor, Capability, Truth, Scenario, Level, Feature**.

---

## 3. Deployment and processes

```text
┌─────────────────────────── Mac (test Mac, always on) ────────────────────────────┐
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

- One `bridge` Go binary, two roles. Self-contained per OS/arch (`darwin/arm64`, `linux/amd64`, `linux/arm64`), pure Go (`modernc.org/sqlite`), cross-compiled on the test Mac.
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
  world/ simapple/ truth/ scenarios/ agent/ evidence/
  truth/native/                     AppleTruth: separate Swift package, shares no code with native/
  scenarios/<source>/*.yaml         scenario definitions
  fixtures/                         scrubbed decoder fixtures
packaging/                          app bundle, Info.plist, entitlements, SMAppService plist, signing scripts
tools/                              lint (zero comments, import boundaries); the feature graph checker and HTML render live in rust/tools/featuregraph
features.yaml                       feature graph
GLOSSARY.md  PRINCIPLES.md  docs/adr/  design/
```

Toolchain on the test Mac: Go 1.27 (Homebrew), Swift 6.3.3 CLT (no Xcode, so no XCTest UI; Swift Testing via `swift test` works with CLT on macOS 26; if not, decoder tests run as an executable target `DecoderChecks` that exits non-zero on failure). Dependencies (keep minimal, permissive licenses): `modernc.org/sqlite`, `github.com/coder/websocket`, `github.com/modelcontextprotocol/go-sdk`, `gopkg.in/yaml.v3`. Swift: system frameworks only (SQLite3, EventKit, Contacts, ApplicationServices, Security, CryptoKit, Network, ServiceManagement).

All builds, tests and services run on the test Mac. `make` targets (Makefile at root): `make build` (Go + Swift), `make lint`, `make features` (checker + HTML), `make sim` (sim-level scenario suite), `make real`, `make replica`, `make agent`, `make dist` (cross-compiled replicas into `bin/`).

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
| messages | `messages.send` (existing chat or handle list; text and/or files), `messages.reply` (threaded reply to a message part), `messages.react` (add/remove tapback or emoji), `messages.edit`, `messages.unsend`, `messages.mark_read`, `messages.mark_unread`, `messages.delete_message`, `messages.delete_chat` |
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

Contacts Me-card and notes entitlement contract: [spec/contacts.md](spec/contacts.md).

Mail capture foundation and local body queries: [Mail](spec/mail.md).

Files source details: [files](spec/files.md).
Safari capture: [design/spec/safari.md](spec/safari.md).

Permission status and explicit requests: [permissions](spec/permissions.md).

Mac capture: [contract](spec/mac.md).
Phone call capture: [source contract](spec/phone.md).

Transient Helper queries and the query-with-effect rule: [query contract](spec/query.md).
Audio query slice: [inputs, bounds and remaining work](spec/audio.md).
Location and Maps query slice: [native queries and limits](spec/maps.md).

Vision and PDF query slice: [inputs, bounds and remaining work](spec/vision.md).

Capture and print provider slice: [capture](spec/capture.md).
Shared helpers for new sources: [SourceSupport](spec/source-support.md).

Relay service and enrollment: [G2-1 / G2-2](spec/relay.md).
OAuth client metadata: [fetch and lab contract](spec/oauth.md).
Source registration: [append-friendly registries](spec/sources.md).
Messages reconciliation: [incremental capture and audit bounds](spec/messages.md).
Shortcut provisioning: [signed builds and source format](spec/shortcuts.md). Clock: [pack qualification and limits](spec/clock.md).
Shortcut provisioning and read-only App Intents discovery: [signed builds, source format and bounded metadata inventory](spec/shortcuts.md). Clock: [pack qualification and limits](spec/clock.md).
Remote onboarding and public readiness: [G1-3 / G1-4 / G1-6](spec/remote.md).

Every source publishes `system.source` with state, coverage, freshness and capabilities, and exposes capabilities individually gated on OS build and proven behaviour. A capability is advertised only after its real-level scenario passed on this OS build (recorded in `features.yaml`); otherwise operations return `ErrUnsupported` with reason `capability_unproven`.

### 9.1 Messages

**Read path.** `~/Library/Messages/chat.db` opened `SQLITE_OPEN_READONLY` with WAL (needs FDA). Schema probed at open (column list hashed into the generation); missing columns become explicit coverage gaps, never crashes. Watch `chat.db`, `chat.db-wal` and the directory with a `DispatchSource` watcher, re-armed after file replacement (inode change), coalesced in bounded 100 ms windows, plus a 2 s polling fallback. Wake and the 15 min timer request reconciliation; an unchanged connection-local SQLite data version and maximum ROWID skip history work. Reopened connections invalidate that version checkpoint. Complete scans use batches of 128 with bounded join materialization and no per-row reload.

**Change algorithm (F8).**
1. Inserts: `message.ROWID > lastRowID` (scan position, database-local).
2. Mutations: rows whose mutable columns changed: `date_delivered, date_read, is_delivered, is_read, date_edited, date_retracted, message_summary_info, error, is_finished, item_type, attributedBody length` — detected by a bounded rescan of the most recent N=2,000 messages plus any message referenced by new rows (`associated_message_guid`, `reply_to_guid`, `thread_originator_guid`) on every change event, plus complete reconciliation after database changes, which hashes rows in bounded batches and suppresses unchanged observations.
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
| `mark_read` / `mark_unread` | Accessibility (Conversation menu) | Incoming rows' `is_read`. Mark as Unread leaves `date_read` and `last_read_message_timestamp` alone, so a read row with `is_read` 0 is what sets chat `is_unread` | |
| `delete_message` / `delete_chat` | Accessibility (context menu → Delete, confirm) | no active membership, with `deleted_at` from the recoverable join; physical absence only after a complete scan | retained recoverable records stay in the copy; test-created data only in tests |

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

Input fingerprint contract: [usage](spec/usage.md).
Action qualification: [usage probe](spec/usage.md).

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
- A single-part `delivery_failed` outcome receives `commit_seq` and `observed_record_ids` only when the same conservative correlation proves a concrete captured failed record. A ghost failure without a matching record remains `delivery_failed` without an ID or commit. A concrete failed ID enables explicit `messages.send` with `resend_of`; neither case automatically redispatches.
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
- Local SSE (`GET /v1/changes?after=`) reuses the same framing semantics for local subscribers, filtered by the caller's Visibility. An oversized capture resets local observers before any part is exposed (`event: reset` with `error: reset` for SSE); clients and internal RYW, status and approval watchers re-read the committed snapshot instead of consuming feed parts.

Budgets (provisional, benchmarked in M0/M5 on the test Mac): 256 KiB frames, 1 MiB blob chunks, 4 MiB credit.

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
- **Packaging (M5):** `Apple Bridge.app` contains the Helper (main executable, UI for onboarding), `bridge` in `Contents/MacOS`, LaunchAgent plist registered with `SMAppService.agent`. Signed app with hardened runtime; notarization is optional, not required. Open-source users may build from source or use right-click → Open for an unnotarized build. Foreground onboarding requests FDA (opens System Settings pane), Automation (Messages, System Events), Accessibility, Contacts, full Calendar, Reminders. Dev builds use a stable self-signed code-signing identity created on the test Mac so TCC grants survive rebuilds.
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

Owns processes, isolated homes (`/tmp/ablab/<world>/…` on each host), ports, network fault proxy (a TCP proxy between replica and gateway owned by the lab that can partition, delay, or cut mid-frame), and artifact cleanup. Hosts: `air` (local when the lab runs on the test Mac) and `ubuntu` (driven over `ssh ubuntu` with a control master). `SimEnvironment` = gateway + SimApplePeer + N replicas, all real binaries. `RealAirEnvironment` = gateway + the real Swift Helper on the test Mac (+ Ubuntu replica for the replica level).

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

`features.yaml` (root) is the DAG feature → requirement → module → scenarios → levels → status → evidence. `rust/tools/featuregraph` (`make features`) checks: every feature has ≥ 1 scenario; every scenario file referenced exists and references its feature; dependencies acyclic; `done` requires every required level's latest evidence passing (`lab/evidence/<level>/<scenario>.json`, redacted); renders `design/features.html`.

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
| CORE-BENCH | modernc SQLite+FTS5+WAL RSS/latency on the test Mac with 200k messages synthetic | S |

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
| MSG-REPLY | Threaded reply to own message part | S R |
| MSG-REACT | Add then remove tapback on own message; emoji reaction | S R |
| MSG-EDIT | Edit own iMessage; edit history visible; 6th edit rejected; >15 min rejected | S R |
| MSG-UNSEND | Unsend within 2 min; retracted visible; > 2 min rejected | S R |
| MSG-MARK-READ | Mark chat read/unread | S R |
| MSG-DELETE | Delete a test message and a test chat; absence published only after complete scan | S R |
| MSG-GROUP-CREATE | Create iMessage group (sim) / RCS group with the authorized group-test participant (real if capability proven) | S R |
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
| USE-LOCK-CLIP | Locked/asleep intervals clipped from Mac totals (sim events; real lock via fixture `pmset displaysleepnow` is not allowed without the owner → sim + historical evidence) | S R |
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
- Decoders: typedstream, message_summary_info, Notes protobuf, SEGB — fixtures captured from this test Mac and scrubbed (`lab/fixtures/`, `native/Tests/DecodersTests`).
- Time math: calendar window chunks, DST, occurrence identity, session pairing and clipping.
- Crash/race timing that can't be triggered from outside: State transaction boundaries (kill points injected through a lab-only build tag), dispatch guard fsync ordering.

If behaviour can be tested from outside, no unit test.

## 20. Quality gates (`make lint`, CI-equivalent run on the test Mac)

- **Zero comments** in Go and Swift source (including generated code and test code): `tools/lint` fails on any `//`, `/*`, `///` comment token (Go via `go/scanner`, Swift via a tokenizer that respects string literals). Build tags use file-name suffixes (`_lab.go`) instead of `//go:build` lines where possible; the only permitted exception is the `//go:build` constraint line when unavoidable, enumerated in the linter.
- Import boundaries per §5.
- `gofmt`, `go vet`, `swift build -Xswiftc -warnings-as-errors`.
- `features.yaml` checker passes.
- Secret scan: no phone numbers/emails/tokens matching patterns in tracked files except synthetic `+1555…`/`@example.com`.

## 21. Milestones

Each milestone proves its riskiest unknown first, starts every feature as a failing E2E scenario, and ends with reviews done, scenarios passing at assigned levels, pushed, PROGRESS.md truthful.

- **M0 — Harness and core at sim level.** Contracts, State, Operations, Replication, Blobs, Trust, Frontdoor (REST+MCP), Runtime basics; `bridge-lab` World/SimApplePeer/SimTruth/Scenarios/FeatureGraph; all CORE-* scenarios at S; CORE-BOOT/RESUME/PAIR/SCOPE-LEAK/CMD-OFFLINE/CMD-CONFLICT/LIVE-LATENCY at P with a Linux replica on Ubuntu. Riskiest first: State snapshot/ApplyBatch correctness and feed gap rules.
- **M1 — Messages, Reminders, Calendar end to end.** Riskiest first: (1) stable dev signing + TCC grant batch on the test Mac; (2) AppleScript send to self on 26.7.1; (3) Accessibility bubble targeting (react/edit/unsend). Then Swift Helper session, AgendaGate, sources, AppleTruth; scenarios sim → real → replica → agent.
- **M2 — Contacts.**
- **M3 — Notes.** Riskiest first: protobuf decode on this test Mac's NoteStore schema; AppleScript write round-trip.
- **M4 — Usage.** Riskiest first: lock/display evidence streams on 25G241.
- **M5 — Packaging and full agent suite.** Riskiest first: signed app, onboarding and background service. Notarization is optional.

## 22. Known limits (truthful, kept current)

- Messages W2 native gaps on this test Mac: pinned order and mute expiry have no proven non-UI source; both remain null with `coverage.missing`. Audio transcription and an explicit Keep decision are not exposed by the observed schema; `kept` remains null rather than inferring Keep from the expirable flag. `schedule.state` preserves the native integer as `native_<n>` until state meanings are independently proven; `scheduled_for` is the message's native date. Recoverable tables expose `delete_date` but no recovery deadline; `deleted_at` is captured and `recoverable_until` remains null with coverage, rather than claiming an exact retention period. Sticker geometry is emitted only where all four stored transform components exist; older sticker records without placement retain `is_sticker`. Unreadable/malformed balloon archives remain typed placeholders with a digest and decode status. Missing native samples for a kind are verified with synthetic fixtures and do not establish real occurrence of that kind.
- Apple arrival lag (iPhone → Mac via iCloud) is outside the < 1 s target; Screen Time remote data lags ~20 h.
- Messages history = what's on the Mac; Messages in iCloud may keep only recent items locally.
- Calendar = ±2 years sliding window; recurrence "all" span unsupported.
- Recurring reminders: next incomplete instance only.
- Contacts notes unavailable without Apple's restricted entitlement; group management later.
- Notes: plain-text edits only; rich edits reject; locked notes title-only.
- Usage: estimates, not Screen Time Settings totals; parity unproven.
- iMessage group features verified at sim level and self-chat where possible; the authorized group-test participant is RCS/SMS.
- Sleep, logout, shutdown, FileVault pre-login are outages.
- Replicas without direct gateway reachability can pair through Remote and use its public Funnel URL (§12, §23.5). Remote must already be enabled on the gateway.
- Local file access on a replica bypasses API scopes; revocation cannot erase copied data.
- Losing `ledger.db` (not just `state.db`) loses the ability to recognize earlier command IDs; recovery requires an explicit operator acknowledgement.
- Packaged gateways use a dedicated `Apple Bridge.keychain-db` in their 0700 application support directory. Installation creates a random 256-bit password in a 0600 `keychain-password` file; the gateway unlocks this Keychain before credential operations, including after login or a crash. Credentials remain Keychain items, with `/usr/bin/security` as their trusted accessor. This permits unattended operation while the login Keychain is locked; possession of the support directory permits unlocking credentials, so it does not protect against another process running as the same user. The user Keychain search list is preserved. Unpackaged CLI use retains the login Keychain default; isolated lab worlds explicitly select file credentials.
- Notarization is optional, not required. Users may build from source or use right-click → Open; optional notarization requires an Apple Developer ID.

---

## 23. Consent and approvals

The full SolidJS web app is preserved on the `web-app` branch. Main has no framework, Node, pnpm, service worker, manifest, browser read APIs or live `web/1` protocol.

### 23.1 Modules

`internal/web` serves Go HTML templates for OAuth consent, enrollment and pending command approvals, plus a handwritten vanilla WebAuthn script and CSS. Operations owns commands and read models; Trust owns passkeys, cookies, agent levels and OAuth credentials. Remote routes REST/MCP unchanged. Agent creation, revocation and snippets use `bridge agent`.

### 23.2 Consent security

The explicit Host allowlist, origin-bound ceremonies, single-use five-minute ceremony IDs, required user verification, CSP script-src 'self', same-origin POST protection, no-store pages, rate limits and session rules remain. Every form POST requires exactly one matching Origin. Cookies are Secure/HttpOnly/SameSite=Strict/Path=/ (`__Host-ab_session`); pinned HTTP loopback alone uses `ab_session_local`. Trusted sessions slide 30 days with a 90-day absolute bound; public sessions expire after 30 idle minutes or 12 hours. Authentication is limited to 10 requests/minute and 100/day/IP with failure backoff and a global 600/minute ceiling. IPv6 groups by /64.

`bridge web enroll --host <origin>` issues a 160-bit, ten-minute, single-use enrollment token and URL/QR. `/enroll#<token>` runs first-passkey registration using `/auth/enroll/options` and `/auth/enroll/finish`. `/auth/login/options` and `/auth/login/finish` run discoverable sign-in. No browser storage holds bearer credentials. `bridge web lock` revokes device sessions.

### 23.3 Pages

`/` is a small server-rendered landing page. `/oauth/authorize` renders the connector name, scopes and read/approve/full level selector with a passkey sign-in button. `/approvals` lists pending commands with redacted arguments, owner-only full arguments and Approve/Reject forms. Approval decisions use the existing durable ledger and resume the normal command lifecycle. iPhone Face ID approvals and delivery remain supported. Removed `/web/v1/*` routes return 404.

### 23.4 Read models

Operations (descriptors in `internal/contracts/operations.go`; available through REST and MCP; scope `messages:read`):

- `messages.chats{filter?: all|known|unknown|unread|deleted|spam|archived, participants?: handle[], limit≤200, after?}` → `{chats: ChatSummary[], next?}`. Ordered by `last_activity_at` desc, then id. Optional `participants` contains 1–32 phone/email handles and matches the exact normalized set (case-folded email and normalized phone; duplicates folded), never a subset or superset. It uses the handle-address and chat-membership indexes in one bounded lookup, applies Visibility, and binds pagination cursors to the normalized set. ChatSummary fields:
  - `chat`: the record
  - `title`: display name, or the participants' contact names joined
  - `avatar_handles`: up to 3 handle ids with `contact_id`
  - `known_sender`: boolean matching the Known Senders server filter: native known classification (including its default), or a non-deleted participant handle with a contact link. Derived from the already-loaded full participant handle set, including members beyond the three avatar handles. Contact visibility does not change this membership bit; it never exposes contact names or IDs.
  - `last_message`: `{id, sent_at, is_from_me, sender_handle_id?, service?, preview{group_event?, kind: text|attachments|reaction|group_event|unsent|edited, text?, attachment_kinds?, count?, reaction?}}`
  - `last_activity_at`, `unread_count`, `has_unseen_mention` (from captured message flags)
  - `muted` and `pinned_index`, nullable where native state is unavailable
  - `pin_preview`: `{message_id, text_preview, at} | null`, the latest unread incoming message with text bounded to 200 Unicode code points plus an ellipsis, maintained by the read model’s existing deltas. REST and MCP consumers receive the same bounded projection.
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

**Unread:** an explicitly marked-unread chat has a minimum summary unread count of one and belongs to the Unread filter. A message counts as unread when it is not from me, has `kind = message`, has `read_at` null, and is not recoverable. An incoming row Messages counts as read with no recorded read time takes `read_at` from `sent_at`. The default chat list excludes recoverable chats; the deleted filter also includes active chats containing recoverable messages. Known includes native known-sender chats and chats with a linked contact; spam selects native Junk chats or chats containing a spam message.

**Implementation:**
- Derived tables in `state.db` (`msg_by_chat`, `chat_summary`, `reaction_by_message`, `attachment_by_message`), updated in the same transaction as each record apply and rebuilt on snapshot install, on the gateway and replicas alike.
- They're never journaled and never authoritative.
- Visibility is applied at read time, as for every read.

**Budgets on the test Mac:**
- `messages.chats` first page ≤ 15 ms p95 at 500 chats.
- `messages.thread` page of 100 ≤ 10 ms p95 on a 170k-message store.

**Reminders read models (W5-STORE; scope `reminders:read`):**

- `reminders.lists{time_zone}` → `{smart: SmartTile[], lists: ReminderList[], tags: TagCount[], cursor}`.
  - `SmartTile`: `{id: today|scheduled|all|flagged|completed|assigned, count?, status?: unsupported}`; incomplete counts except Completed. Today includes overdue incomplete reminders and reminders due during the viewer's day. Scheduled has a due date and is incomplete. Flagged requires the native store flag. Assigned means assigned to the current user, never merely assigned to someone. Its predicate uses the source-verified `assignment.assignee.is_me` boolean; unknown current-user identity makes the Assigned tile `status=unsupported` with no count, just like an unsupported custom smart filter. Never infer identity from a display name. Each predicate has a synthetic-store scenario assertion.
  - `ReminderList`: `{list: Record, count?, status?: unsupported, sections: ReminderSection[], group_id: id|null, order, pinned}`. This is a flat sidebar projection; groups are list records with `is_group=true`, and each child names its group in `group_id`. Return pinned entries first in native pin order, then native display order, with record id only as a tie breaker. Consumers receive the native order and group IDs. Custom smart lists get a count from a supported decoded filter, or `status=unsupported` with the count omitted.
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
| `/`, `/enroll`, `/auth/*`, `/approvals`, `/oauth/*`, `/.well-known/oauth-*` | Web | device session / OAuth |
| `/v1/*`, `/mcp` | Frontdoor (public-host mode) | `Authorization: Bearer` with remote-allowed principals: **agent** tokens (§23.6), or explicitly opted-in `bridge token create --remote` tokens during W1; any `Origin` header rejects; local machine tokens are refused remotely |
| `/v1/feed` | Replication on gateways; 404 on replicas | `relay/1` paired device-key authentication (§12); any `Origin` header rejects |

TLS ends on the Mac: inside `bridge` with tsnet in embedded mode, or in system tailscaled in system mode, using the ts.net certificate. Funnel relays cannot read traffic. Remote pins the public Host before dispatch. System mode recognizes HTTPS and forwarded source only through its dedicated capability-protected mapping; forwarded Host must exactly match the configured public host, forwarded Proto must be HTTPS, and forwarded source must be one valid IP. The shared Web handler sees the pinned public Host, HTTPS, and original client IP. Embedded mode extracts the original source from the TLS connection’s `ipn.FunnelConn.Src`. Local API requests never inherit proxy trust, and ignore forwarded headers. Bandwidth limits are Tailscale's (unpublished), so media variants (§23.7) keep transfers small.

### 23.6 Agent access

Agent tokens retain read, approve and full levels, grants, revocation and the 90-day activity log. Approve-level commands wait for owner approval; rejecting finalizes rejected_by_owner. Approval lasts 24 hours and the effect TTL starts at approval. CLI agent management includes connection snippets.

MCP OAuth 2.1 retains protected-resource and authorization-server discovery, dynamic client registration, authorization and token endpoints. PKCE S256 and resource binding are mandatory; access tokens last one hour, refresh tokens rotate and last 90 days. Revoking the agent revokes connector access. Consent uses a passkey-authenticated device session and selected level/scopes. Client metadata document validation remains.

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
