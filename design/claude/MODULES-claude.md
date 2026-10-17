# Apple Bridge module architecture (Claude, independent of Astra's)

Vocabulary from Matt Pocock's codebase-design: module, interface, implementation, depth, seam, adapter, leverage, locality.

## 1. Glossary

| Term | Meaning | Avoid |
|---|---|---|
| **Gateway** | The `bridge` program in gateway mode on the Mac. The only sequencer and command authority. | hub, server, master |
| **Replica** | The `bridge` program in replica mode on any machine. Holds a full local copy, serves reads, forwards commands. | client, mirror node, slave |
| **Helper** | The Swift process on the Mac that does all Apple I/O. Stateless. | agent, daemon, adapter (that word is reserved for seams) |
| **Source** | One Apple data domain inside the Helper: messages, calendar, reminders, contacts, notes, screentime. | connector, plugin |
| **Record** | One normalized item: `{collection, id, version, deleted, data}`. | row, object, entity |
| **Collection** | A named record type, e.g. `messages.message`. | table, type |
| **Journal** | The gateway's ordered, durable log of record changes, each with a `seq`. | change log, event store |
| **Epoch** | Identifies one continuous journal history. A new epoch forces replicas to resnapshot. | generation (reserved for replica store generations) |
| **Cursor** | A replica's last applied `(epoch, seq)`. | offset, position |
| **Source cursor** | A Helper-side resume token per Source (chat.db generation+rowid, Contacts history token, ...). The gateway owns its persistence. | bookmark |
| **Command** | A requested write: `{id, op, args, expected_version, deadline}`. | mutation, action, request |
| **Outcome** | The final state of a command: `succeeded`, `failed`, `expired`, `rejected`, `unknown`. | result, response |
| **Evidence** | Facts recorded before and after dispatch that let an `unknown` be resolved. | proof, receipt |
| **Principal** | An authenticated caller: a paired replica or an API token, with scopes. | user, client |
| **Catalog** | The single registry of collections, operations, scopes and capabilities, with versions. | schema registry, spec |
| **Coverage** | What a Source can actually see and do on this Mac, published as data. | support matrix |
| **Truth** | Apple's own state, read by the harness without going through the bridge. | ground truth (fine), reality |

## 2. Shape

```
                ┌──────────────── Mac ────────────────────────────────────────────┐
 Apple apps ◄──►│ Helper (Swift)  ══ Helper Protocol (UDS) ══  bridge gateway (Go)│◄═ Feed ═► bridge replica (Go)
                └──────────────────────────────────────────────────────────────────┘               any machine
```

Three real seams carry everything. Each has two adapters:

| Seam | Adapter A | Adapter B |
|---|---|---|
| **Helper Protocol** (Helper ↔ gateway) | Swift Helper (real Apple) | SimHelper (Go, fake Apple, fault knobs) |
| **Source interface** (inside Helper) | 6 Sources | (6 adapters, real) |
| **Commands interface** (inside bridge) | Local ledger (gateway) | Forwarder (replica → gateway over Feed) |
| **Surface** (outside world) | REST + SSE | MCP |

Everything else is an internal module with one implementation; we don't invent seams for it.

## 3. Module map

### Mac side (Swift package `mac/`)

**M1. Helper** (process entry). Interface = the **Helper Protocol** (§4.1). It hides process lifecycle, permission onboarding prompts, socket auth (peer UID + code-signature check of the gateway binary), routing to Sources and backpressure. Stateless across restarts: all cursors and command ledgers live in the gateway. Tested via E2E at the real level only (sim swaps the whole process).

**M2. Source** (internal seam, 6 adapters). Interface:
```swift
protocol Source {
  var name: SourceName { get }
  func coverage() async -> Coverage
  func snapshot(into sink: BatchSink) async throws -> SourceCursor
  func follow(from: SourceCursor, into sink: BatchSink) async throws
  func execute(_ c: Command, evidence: EvidenceSink) async -> Outcome
  func resolve(_ c: Command, evidence: [Evidence]) async -> Outcome
}
```
Invariants: a snapshot ends with a `complete` marker. Deletions are emitted only after a successful complete scan of that scope. Permission loss emits `health(permission_missing)`, never tombstones. `execute` records pre-dispatch evidence before the side effect.

- **MessagesSource**: hides the chat.db read-only WAL reader, file-event watching + bounded rescan + periodic full reconcile, typedstream decoding, GUID identity, attachment staging, and a **MessagesActuator** with two strategies (AppleScript for basic sends; Accessibility automation in the Beeper style for edit/unsend/react/reply/group ops). Outcome matching by fingerprint + candidate rows.
- **CalendarSource / RemindersSource**: share an internal **EventKitGate** (authorization, debounced `EKEventStoreChanged`, hashed diffing, chunked ≤4-year queries, sliding window, identity aliases scoped by account/calendar, `occurrenceDate`-based occurrence identity).
- **ContactsSource**: `CNChangeHistoryFetchRequest` with token persistence via the gateway; handles drop-everything resets; writes via `CNSaveRequest`.
- **NotesSource**: NoteStore.sqlite protobuf reader; AppleScript writes; rejects unsupported rich edits.
- **ScreenTimeSource**: Biome SEGB reader (local + remote devices), device names from `DevicePeer`, lock/sleep clipping, read-only. Emits per-session and per-day records flagged `estimated`.

Code-level tests here only for decoders (typedstream, SEGB, Notes protobuf) against fixtures captured from the Air, and for time math (windowing, DST, occurrences, session clipping).

### Go side (`bridge/`, one binary, two modes)

**B1. Catalog.** Interface: typed descriptors for every collection, field, operation, scope and capability, with protocol/schema/operation versions. Source of truth is `catalog/*.json` (JSON Schema). Generates Go and Swift types and Surface wiring. Hides versioning and compatibility rules (unknown-field and enum handling). Tested through E2E scenarios that hit every operation, plus a generated-code drift check.

**B2. Journal** (gateway only). Interface:
```go
type Journal interface {
  Apply(ctx, SourceBatch) (Seq, error)
  Head() Position
  Snapshot(ctx, filter Scope) (SnapshotReader, error)
  Since(ctx, Position, filter Scope) (ChangeStream, error)
  SourceCursor(SourceName) (SourceCursor, bool)
}
```
Hides atomic record+log+source-cursor commits, version bumps, tombstones and their collection, consistent snapshots (one read transaction pinned at a seq), retention, epoch rotation on discontinuity, and backfill-must-not-overwrite-newer rules. This is the deepest Go module. Tested through sim E2E: kill mid-apply, snapshot during heavy change, retention expiry, restored backup.

**B3. Commands.** Interface (real seam, two adapters):
```go
type Commands interface {
  Submit(ctx, Principal, CommandRequest) (CommandStatus, error)
  Status(ctx, Principal, CommandID) (CommandStatus, error)
  Watch(ctx, Principal, CommandID) (<-chan CommandStatus, error)
}
```
- **Ledger** (gateway): idempotency by `(principal, id)` + canonical request hash, mismatch rejection, expiry and authorization rechecked immediately before dispatch, per-object serialization, expected-version checks, claim → evidence → dispatch → outcome. `unknown` resolution via `Source.resolve`, never auto-resend. Returns the journal `seq` at which the effect became visible. Fails fast when the Helper is unavailable.
- **Forwarder** (replica): sends over the Feed connection, recovers indeterminate responses by the same ID, returns `unavailable` when disconnected (no queue in v1).

**B4. HelperLink** (gateway only). Interface: `Run(ctx)` plus `Dispatch(Command) Outcome` and `Coverage()`. Hides spawning/supervising the Helper (or SimHelper), socket auth, protocol version negotiation, handing source cursors back on start, feeding batches into Journal, and health. Tested through both E2E levels; the SimHelper is its second adapter.

**B5. Store** (every machine). Interface:
```go
type Store interface {
  Get(ctx, Scope, Collection, ID) (Record, error)
  Query(ctx, Scope, Query) (Page, error)
  Search(ctx, Scope, string) (Page, error)
  Subscribe(ctx, Scope, Position) (ChangeStream, error)
}
```
The read model: SQLite + FTS5, per-collection indexes declared by the Catalog, documented read-only SQL views as a versioned contract. On the gateway the Journal writes into it; on a replica the Mirror does. **Same read path on every machine**, which is what makes "same API everywhere" true.

**B6. Feed** (gateway server, replica client). Interface on the replica side: `Mirror.Run(ctx, Peer)`. It hides the WebSocket protocol, hello and capability negotiation, snapshot staging into a new store **generation** with an atomic swap, durable acknowledgments, ordered batches, heartbeats, slow-consumer cutoff and resnapshot, scope filtering, and multiplexing the command channel for the Forwarder. Tested with sim E2E plus network faults through a proxy.

**B7. Blobs.** Interface: `Put/Open/Has(hash)` plus `Want(hash)` progress. It hides content-addressed storage, chunked resumable transfer over the Feed, digest checks, size caps, disk-full behaviour and reference-counted GC. Attachment records carry `blob_state` (`present | pending | unavailable | over_cap`).

**B8. Access.** Interface: `Pair(code)`, `Authenticate(req) Principal`, `Allow(Principal, Action, Scope) bool`, `Revoke(id)`. It hides device keys, expiring rate-limited pairing codes, scope algebra, localhost auth and DNS-rebinding defense, and audit. **Every** data path takes a Scope from Access: snapshots, changes, queries, search, blobs, commands, command results. Tested via E2E scope-leak scenarios (a reminders-only token can't see a message anywhere).

**B9. Surface.** Two adapters over one operation table generated from the Catalog: **REST + SSE** (`/v1/...`, OpenAPI) and **MCP** (Streamable HTTP + stdio). It hides HTTP and MCP details only; all logic lives in Store/Commands. Tested by the scripted client (REST) and the agent level (MCP).

**B10. Health.** Interface: publishes `system.source`, `system.replica`, `system.command` collections (scope-filtered). It hides measurement of capture lag, apply lag, coverage, oldest pending command, unknown counts and blob backlog. Replicas compute their own disconnection staleness locally.

### Harness (`e2e/`, Go)

**H1. World.** Interface: `Up(spec)`, `Down()`, `Fault(kind, target)`, `Clock(adjust)`. It hides starting processes locally and over SSH (Air, Ubuntu), isolated data dirs, a fault proxy between gateway and replicas, and kill/pause/restart. This is the "lever" agents use to drive the system (JSON output).

**H2. SimHelper.** The second adapter of the Helper Protocol: an in-memory fake Apple with a control API (`inject`, `inspect`, `faults`). It implements every Source op, including deliberately nasty behaviours (crash after effect, duplicate events, permission loss, schema change).

**H3. Truth.** Interface: `Observe(query) Fact`. Adapters: SimTruth (asks SimHelper) and AppleTruth (a tiny separate Swift oracle binary reading chat.db/EventKit/Contacts directly, plus Messages-window screenshots for UI-visible effects). It never shares code with the Helper, so a Helper bug can't hide itself.

**H4. Scenarios.** Declarative YAML steps (`given/when/then`), levels `[sim, real]`, standard automatic assertions (convergence, latency, exactly-once, no-false-delete, no-scope-leak). The runner records p50/p95/max on one clock via replica Subscribe streams.

**H5. AgentRunner.** Runs a GPT-6-Luna agent (Codex) on Ubuntu with only the MCP, executes natural-language tasks 3×, and grades on Truth + replica state, never on the agent's claims.

**H6. FeatureGraph.** `design/features.yaml` (feature → requirement → module → scenarios → status), a checker that fails on unlinked items, and an HTML graph render.

## 4. Key seams in detail

### 4.1 Helper Protocol: design it twice

**Option A: "chatty RPC".** The gateway calls fine-grained methods (`listChats`, `messagesSince(rowid)`, `sendText`, `addReaction`, ...). Shallow: the gateway has to know every Apple quirk, and the seam grows with every feature.

**Option B: "batches and commands" (chosen).** Five message kinds, independent of source:
```
gateway → helper : Start{source, cursor?}  Execute{command, evidence_ref}  Resolve{command, evidence[]}
helper → gateway : Batch{source, kind: snapshot|change|complete, records[], tombstones[], cursor}
                   Outcome{command_id, state, evidence[], record_refs[]}  Health{source, coverage, state}
```
Length-prefixed CBOR over a Unix domain socket, with version negotiation in a Hello. New Apple features add Catalog collections and operations without changing the seam. Its depth: the gateway learns 6 message kinds and gets 6 Sources' worth of behaviour.

### 4.2 Feed protocol
`Hello{epoch, cursor, scope_version}` → `Snapshot{generation, at_seq, pages...}` | `Changes{from, to, batch}` → `Ack{seq}`; `Heartbeat`; `Command/CommandStatus` multiplexed. Replicas apply each batch and its cursor in one transaction.

## 5. Dependency graph (no cycles)

```
Surface → Store, Commands, Access, Catalog
Commands(Ledger) → HelperLink, Journal, Access
Commands(Forwarder) → Feed(client)
HelperLink → Journal, Catalog
Journal → Store, Catalog
Feed(server) → Journal, Access, Blobs
Feed(client)/Mirror → Store, Blobs
Health → Journal, HelperLink, Store
Helper(Swift) → Sources → (EventKitGate, decoders)
Harness → World, SimHelper, Truth (never imports bridge internals; talks only through public surfaces)
```

## 6. Repo layout

```
apple-bridge/
  catalog/              JSON Schema source of truth + codegen
  mac/                  Swift package: Helper, Sources, decoders, AppleTruth oracle (separate target)
  bridge/               Go module: cmd/bridge, catalog, journal, commands, helperlink, store, feed, blobs, access, surface, health
  e2e/                  world, simhelper, truth, scenarios/*.yaml, agent, measure
  design/               SPEC.md, GLOSSARY.md, adr/, features.yaml, research/
  tools/                comment-ban lint, import-boundary checks, feature-graph checker
```

## 7. ADRs

1. Helper is stateless; the gateway owns all durable state (cursors, ledger).
2. The Helper Protocol is batches + commands, not fine-grained RPC.
3. One `bridge` binary with gateway and replica modes.
4. Store is the single read path on every machine.
5. Commands is a real seam: Ledger (gateway) and Forwarder (replica).
6. Catalog in JSON Schema is the source of truth; code is generated.
7. Feed is a custom WebSocket protocol, not NATS (one sequencer, simple topology; revisit if fan-out grows).
8. Snapshot = pinned read transaction + staged replica generation + atomic swap.
9. Writes fail fast; there are no offline queues in v1.
10. Messages advanced actions use Accessibility automation; SIP stays on.
11. Screen Time comes from Biome App.InFocus, is read-only, and is flagged as estimated.
12. Zero comments, enforced by lint; import boundaries enforced by tooling.
13. Truth oracles share no code with the Helper.
14. Public contracts (REST, MCP, Feed, Helper Protocol) are versioned; internals are not.

## 8. Risks to modularity

- **Messages complexity leaking into the gateway.** Keep all Apple semantics in MessagesSource; the gateway sees only records and commands.
- **Catalog becoming a shallow pass-through.** It must own versioning, scope and index declarations, not just types.
- **Surface growing logic.** Lint: Surface may import only Store/Commands/Access/Catalog.
- **SimHelper drifting from reality.** Every real-level scenario also runs at the sim level, and any divergence found at the real level becomes a SimHelper fix plus a scenario.
- **Accessibility automation fragility.** Isolated behind MessagesActuator, with capability flags in Coverage so callers see what works on this OS build.
