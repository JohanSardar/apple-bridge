# Apple Bridge: independent module architecture

Proposal for implementers; no implementation authorized. [Decisions](../DECISIONS.md), [resolution](../debate/RESOLUTION.md), and the [20 reliability findings](../claude/astra-critique-of-claude-r1.md) override superseded choices in [my earlier design](DESIGN-astra.md). Only the permitted critique was read under `claude/`; `design/modules/` was not read.

SIP stays on; no iPhone app. “100%” means equality within declared available scope, with separate metadata/blob coverage. Messages imports local history without forced iCloud hydration; the Calendar window slides daily. Source choices follow the supplied [Messages](../../research/imessage.md), [Apple-data](../../research/apple-data-and-sync.md), [Screen Time](../../research/screentime-sip.md), and [pstack](../../research/poteto-principles.md) research; the real probe outranks earlier speculation.

## 1. Glossary

Apple Bridge exposes the Mac's authorized Apple data and verified effects to other computers.

**Source**: An Apple data domain available through the Mac. _Avoid_: backend, provider.

**Gateway**: The Mac's sole bridge sequencer and command authority. _Avoid_: master, cloud.

**Replica**: A durable local copy of an explicitly authorized bridge scope. _Avoid_: cache, mirror client.

**Bridge record**: An identified representation of an observed Apple object. _Avoid_: native row, document.

**Observation**: Evidence of source state, including absence when established. _Avoid_: effect, notification.

**Command**: A request for an Apple effect, distinct from its observation. _Avoid_: write, job.

**Coverage**: The records, fields, operations, history, and attachments represented, including known gaps. _Avoid_: completeness, synced boolean.

**Freshness**: How recently source state was observed and propagated. _Avoid_: health, connectivity.

**Source checkpoint**: Capture progress within one source generation. _Avoid_: replica cursor.

**Replication cursor**: Position within one gateway history and subscription scope. _Avoid_: timestamp, row ID.

**Locally observed**: Evidence of an effect in the Mac's source state. _Avoid_: delivered, successful send.

**Indeterminate**: Acceptance or effect cannot yet be established. _Avoid_: failed, safe to retry.

## 2. Module map

The interface is the test surface. Dependency categories below use the method's names. **One adapter = hypothetical seam, two = real.** Concrete modules retain interfaces without speculative substitution ports. `sim` means fake Apple behind IPC, all Go, storage, network, and replicas real; `real` adds the actual Swift helper and Apple apps. Categories describe dependencies, not deployment.

Common interface contract: typed errors distinguish unsupported, unauthorized, unavailable, conflict, reset, expired, corrupt, capacity, and indeterminate. Bounded operations accept cancellation/deadlines; cancellation never reverses dispatched effects. No cross-source ordering is promised before gateway sequencing. Live metadata has an awake, healthy-path <1 s capture-to-replica target; Apple arrival lag and blob completion are measured separately.

### Swift

**AppleSession** — Interface: the authenticated duplex IPC below. Its implementation hides process fencing, capture scheduling, bounded buffering, native preparation, one-use dispatch tickets, and a durable dispatch-receipt guard. One helper execution owner; unfinished dispatches become indeterminate after crashes. One UI actor serializes Accessibility actions, verifies targets, and aborts on human interference. Live observations outrank backfill/blobs. Seams: `ApplePeer` has **SwiftHelper / SimApplePeer** adapters; internal `Source` has **Messages / Agenda / Contacts** adapters. Categories: **Remote but owned**, **True external**, **Local-substitutable** receipts. Tests: sim disconnect/duplicate/ambiguity/backpressure; real permission, UI-interference, and helper-restart scenarios.

**Messages** — Interface: `Source` observations plus typed send/edit/unsend/react/reply/group actions, individually capability-gated by OS build, iMessage/RCS/SMS route, iPhone-relay prerequisites, and native action windows. Existing conversations, new recipients, new groups, and each group-management action are distinct. Its implementation hides read-only WAL SQLite, schema probes, GUID aliases, multipart typedstream decoding, rearmed sidecar watchers, bounded rescans/polling fallback, complete reconciliation, and AppleScript/Accessibility. Undecoded payloads retain bytes and status. Text/file sub-effects retain separate outcomes; zero native error never proves delivery. **True external**; real `Source` adapter, no extra mock port. Tests: real self-messages, mixed attachments, mutations, receipts, deletion, ghost sends; decoder fixtures cover Unicode and malformed bodies. New-group capability waits for separate proof.

**Agenda** — Interface: `Source` for Calendar and Reminders; create/patch/delete, recurrence spans `thisEvent`/`futureEvents`, and completion. Hides EventKit invalidation/refetch, account writability, chunked sliding ±2-year queries, account/calendar-scoped native aliases, original occurrence dates, exceptions, all-day/timezone semantics. Window eviction differs from deletion; recurring reminders expose only the obtainable next incomplete instance; unsupported “all” span rejects. **True external**; shares the real `Source` seam, no EventKit mock. Tests: real CRUD, DST, moved occurrences, split series, permission loss; pure time-math fixtures where E2E cannot reach.

**Contacts** — Interface: `Source` observations and basic create/update/delete via CNSaveRequest in v1. Fetch unified contacts, group membership, names, phones, emails, addresses, organization, birthday, and photos; restricted notes remain unavailable without entitlement. Hides native identity/aliases, writability, and history tokens. Changes and token travel together; `DropEverything` requests reconciliation, never deletion. Unavailable keys remain explicit; group management comes later. **True external**; real `Source` adapter. Tests: real CRUD/read-only accounts; sim reset and token-crash scenarios through IPC.

**Notes** — Interface: `Source`, demonstrated read/create/patch operations after the core. Hides consistent WAL-aware copies, schema/protobuf decoding, and AppleScript. Locked/offloaded content stays explicit; unsupported rich edits reject before dispatch rather than flattening documents. **True external**; real `Source` adapter, no speculative alternate writer. Tests: real supported round-trips, locked/rich rejection; decoder fixtures.

**Usage** — Interface: read-only `Source` app/device/day estimates with provenance, source timestamps, uncertainty, and decoder version. Hides Biome SEGB decoding, DevicePeer mapping, focus pairing, and lock/display clipping where evidence exists; knowledgeC is corroboration, never blindly summed. [The Air probe](../../research/screentime-probe-results.md) found readable remote iPhone focus, ~20-hour lag, inflated Mac totals, and inaccessible ScreenTimeAgent stores with SIP on. Settings parity remains unproved. **True external**; real `Source` adapter. Tests: real per-device provenance/Settings comparison; decoder/time fixtures and sim delayed arrivals. No write operation.

### Go: one binary, gateway or replica role

**Contracts** — Interface: generated tagged records/operations, wire codecs, structural validation, and explicit query/index descriptors. Hides language representation differences. Wire, collection, and operation versions evolve separately; preserve unknown optional fields, reject unknown required semantics without advancing checkpoints. **In-process**; no adapter. This deliberately small module earns locality, not artificial depth. Tests: sim rolling upgrades; decoder fixtures. Semantic operation policy is handwritten.

**State** — Interface: `CommitCapture`, `TransitionCommand`, `Read`, `FreezeSnapshot`, `InstallSnapshot`, `ApplyBatch`. Hides SQLite/FTS, aliases, generations, migrations, journal retention, and command storage. Capture commits records+aliases+journal+checkpoint atomically. Command transitions use compare-and-set; ledger/identity retention is independent of mirror rebuilds. ID/hash deduplication tombstones survive their admission era. Snapshots freeze at S, pin later history, stage separately, validate count/digest, and atomically replace generation+cursor, removing absent records. Apply rejects gaps and commits data+cursor before acknowledgment. **Local-substitutable**: isolated real SQLite files; one adapter, deliberately no repository port. Tests: sim interrupted snapshots, resets, disk-full, restore; precise crash/race tests only for unreachable transaction timing. Short transactions and bounded pages protect live work.

**Operations** — Interface: `Read(Query)`, `Submit(Command)`, `Status(CommandID)`, `Observe`; same semantics at gateway and replica. Hides capture-to-State ingestion, useful searches, write validation, native refresh, field-patch conflicts, per-object/series serialization, durable execution claims, and conservative result correlation. Updates/deletes require expected bridge versions; conflicts return current state. IDs bind authenticated principal+canonical request hash; mismatches reject. Persist route/fingerprint/candidate IDs before dispatch; recheck authority and server-UTC expiry immediately before dispatch, with operation-specific maximum TTL and monotonic in-process timers. Reject excessive clock skew; revalidate after wake/restart. Offline submission fails fast; lost responses require original-ID lookup. Never automatically redispatch uncertain effects. Stronger evidence can resolve indeterminate outcomes while preserving audit history. Status distinguishes local observation/delivery/failure, including commit cursor for local waiting. External Apple writers can still race. **Local-substitutable / Remote but owned**; `CommandTarget` adapters **LocalGateway / RemoteGateway**, plus `ApplePeer`. Tests: sim competing edits, identical/manual sends, response loss, clock jumps; real observed values/absence.

**Replication** — Interface: `Follow(subscription,cursor)`, `Serve`, `Wait(cursor)` using the protocol below. Hides WebSocket reconnect, catch-up/live transition, snapshot leases, ordered filtered watermarks, and credit control. Replicas never poll; resume/acknowledgment are control traffic. Restore rotates history epoch; out-of-range cursors resnapshot. Tombstones retire only with retained-history/snapshot guarantees. Slow consumers disconnect before exhausting bounded memory. Local SSE reuses framing/application semantics. **Remote but owned**; live **WebSocket / SSE** adapters share ordered-feed semantics; replica transfer uses WebSocket. Tests: sim partitions, retention races, scope changes, restored backup, concurrent bootstrap/live changes; real Air→Ubuntu latency. No fake network.

**Blobs** — Interface: `Stage`, `OpenAuthorized(recordGrant,digest)`, `Transfer`, `Protect(referenceSet)`, `Progress`. Hides immutable content-addressed files, secure staging, chunk/resume/digest checks, quotas, and reference-aware collection. Authorized State reads supply record grants; digest knowledge grants nothing. Operations/Replication supply complete State reference manifests protecting records, snapshots, and commands; staging/transfer pins precede record commits and remain until reference protection is durable. All attachments download by default up to each machine's cap; offloaded, missing, capped, and disk-full remain distinct from metadata completeness. Live work preempts bulk chunks. **Local-substitutable / Remote but owned**; local files, ApplePeer reads, and authenticated chunk transport are concrete dependencies, no unused storage port. Tests: sim interruption, corruption, capacity, reference deletion; real attachments.

**Trust** — Interface: `Pair`, `Authenticate`, `Authorize`, `Visibility`, `Revoke`. Hides expiring rate-limited pairing codes bound to device keys, rotation, credentials, and subscription revisions. Replica identities differ from scoped callers; forwarded commands carry gateway-verifiable delegated identity. One visibility policy governs snapshots, changes, searches, blobs, commands/results, and MCP. Scope changes require purge/backfill in a new subscription generation. **In-process / Local-substitutable / True external** credential stores; **Keychain / LinuxSecretStore** adapters make that internal seam real. Tests: sim cross-scope leakage, forged delegation, revocation, pairing replay; real credential lifecycle. Revocation cannot erase copied data; local file owners bypass caller scopes.

**Frontdoor** — Interface: versioned REST and MCP operations, authenticated even on localhost; push status/subscriptions. Hides protocol translation, bounded parsing, origin/Host checks against browser-origin/DNS-rebinding attacks, rate limits, and redacted audit output. **REST / MCP** are adapters onto the same Operations interface; no duplicated semantic validators. **Remote but owned / In-process**. Tests: every sim journey through both, real scripted client and remote MCP-only agent; unauthorized input must fail before effects.

**Runtime** — Interface: `start`, `pair`, `doctor`, `upgrade`, `restore`, `stop`, reporting readiness/staleness rather than just process liveness. Hides role wiring, singleton locks, migration/backup sequencing, and Swift/Go compatibility. Restore stays read-only until fresh history/admission eras are installed; every history discontinuity rotates epoch. Mac delivery is one signed/notarized app with stable identity, foreground FDA/Automation/Accessibility/Contacts/full-Calendar/Reminders onboarding, SMAppService, and supported sleep assertion. Require encrypted disks/backups, restrictive files including WAL/indexes/blobs/ledger, protected keys, and TLS/device authentication. Logout, sleep, and FileVault pre-login remain outages; replicas detect disconnect independently. **Local-substitutable / True external**; concrete platform installation, no installer abstraction. Tests: real packaged background execution, upgrade, login/wake; sim backup rollback. Benchmark modernc SQLite, FTS5, WAL, RSS on the 8 GB Air before packaging claims.

### Harness

**Control** — Interface: `bridge-lab start|act|await|fault|assert|cleanup|run`, emitting structured evidence. Hides process/network orchestration, seeded fake Apple state, artifact ownership, deadlines, and repeatable cleanup. `Environment` adapters **SimEnvironment / RealAirEnvironment**; `Actor` adapters **ScriptedClient / CodexMCPAgent**. **Local-substitutable / Remote but owned / True external**. Both levels exercise product interfaces; faults kill/partition/fill resources rather than edit product tables. Real writes use isolated owned fixtures on Johan's account, self-messages, and authorized Ben RCS/SMS groups; Ben cannot certify iMessage groups. Build on the Air; Ubuntu runs the replica and GPT-6-Luna via Codex with only MCP. Tests begin as failing E2E scenarios; novel failures extend the plan. `features.yaml` is a dependency DAG linking capability, module, scenarios, environment, and evidence. Measure capture/apply lag, coverage, drift, command age/unknowns, blob backlog, and multi-year reconciliation cost. The harness is also the operator's control tool.

## 3. Dependency graph

Arrows mean implementation depends on interface; responses do not add reverse edges. Contracts is the common leaf; external Apple/OS dependencies are omitted.

```text
Control → Runtime, Frontdoor, Contracts
Runtime → Frontdoor, Operations, Replication, State, Blobs, Trust, AppleSession
Frontdoor → Operations, Replication, Blobs, Trust
Operations → State, Blobs, Trust, ApplePeer[Contracts]
Replication → State, Blobs, Trust
Blobs → Trust, ApplePeer[Contracts]
State → Trust, Contracts
Trust → Contracts
AppleSession → Messages, Agenda, Contacts, Notes, Usage, Contracts
Messages, Agenda, Contacts, Notes, Usage → Contracts
```

All modules depend on Contracts. Runtime injects ApplePeer; Go never imports native modules. RemoteGateway is a CommandTarget adapter using the wire contract, not a dependency back onto Frontdoor. Status publication follows State's journal, not callbacks into Operations.

## 4. Concrete seams

### ApplePeer: design it twice

**A: resource RPC.** `BeginScan → ReadPage → CommitCapture`; independently `WatchHints`, `Prepare`, `Dispatch`, `Resolve`, `ReadBlob`. Flexible, but Go must learn native scan choreography; depth and locality suffer.

**B: duplex conversation, selected.** Swift owns capture progress; Go learns durable facts and execution evidence. A considered immutable-export alternative adds leases to every live change; use that approach only for bridge snapshots.

```text
Open(protocol, ownerFence, checkpoints, byteCredit)
Ready(capabilities, coverage)
Capture(source, generation, predecessor, checkpoint, observations,
        scanID, scope, completionEvidence)
Committed(source, checkpoint)
Prepare(commandID, principal, requestHash, action, expectedState, expiresAt)
Prepared(ticket, nativeState, route, fingerprint, candidateIDs)
Dispatch(ticket, ownerFence)
Outcome(commandID, disposition, evidence)
Resolve(commandID)
Blob(handle, offset, length) → Chunk(bytes, digest)
```

Length-framed typed messages over an owner-only Unix socket with peer identity plus installation-secret authentication. Paired releases negotiate versions before effects. Go acknowledges capture only after atomic commit. Source generations reset capture identity, not gateway history. Only successful complete scoped scans justify absence; revalidate candidates and protect records updated after scan start. Backfill revalidates before emitting. Permission loss never completes an empty scan.

Tickets are expiring, single-use, principal/hash-bound, and session-fenced; prepare reserves the execution slot. Dispatch rejects busy/expired tickets instead of queuing beyond authorization. Swift rechecks native state/deadline and persists dispatch-start before acting; unfinished starts yield indeterminate. Apple effect→result receipt remains an unavoidable ambiguity window. Restore also rotates command-admission era: earlier IDs permit lookup, never new execution.

### Source: native lifecycle, not generic CRUD

```swift
protocol Source {
    func capture(_ resume: Checkpoint?, into: ObservationSink) async throws
    func prepare(_ command: NativeCommand) async throws -> Prepared
    func dispatch(_ prepared: Prepared) async -> Evidence
    func resolve(_ command: CommandID) async -> Evidence
}
```

Typed action unions retain domain distinctions; read-only adapters reject preparation. Capture sinks apply credit limits. A source fault cannot masquerade as an empty result. Native references stay behind IPC except opaque scoped aliases; State assigns stable bridge IDs.

### Replica feed

```text
Cursor = (epoch, subscriptionRevision, sequence)
Resume(cursor, schemaOffer, credits)
SnapshotBegin(id, S, scope, count, digest, expiresAt)
SnapshotPage(id, ordinal, records)
SnapshotEnd(id)
Batch(epoch, subscriptionRevision, after, through, changes)
Ack(cursor)
Reset(reason, retentionFloor)
```

`(after,through]` covers every sequence, including filtered-out changes; empty batches advance watermarks. Ignore batches wholly behind the durable cursor; otherwise require `after == durableCursor.sequence`. Gaps/partial overlaps never advance. Resume requires `retentionFloor ≤ cursor ≤ head` in the same epoch/subscription. Snapshot installation at S precedes replay of retained S+1 onward; replay and live reads use the same journal. Expired/interrupted staging is discarded or resumed only under its valid lease. Versions never compare across epochs. Start with 256 KiB metadata frames, 1 MiB blob chunks, and 4 MiB outstanding credit to bound Air memory; benchmark these provisional budgets. Configure finite journal/disk caps and snapshot leases; eviction forces resnapshot.

## 5. Repository layout

```text
contracts/                         Contracts; generated Go/Swift shapes
native/AppleBridge/Session/         AppleSession
native/AppleBridge/Sources/{Messages,Agenda,Contacts,Notes,Usage}/
internal/{state,operations,replication,blobs,trust,frontdoor}/
cmd/bridge/, internal/runtime/, packaging/   Runtime and composition
cmd/bridge-lab/, lab/               Control
lab/{scenarios,fixtures,evidence}/  E2E definitions and redacted results
features.yaml                      Feature/scenario dependency graph
GLOSSARY.md, docs/adr/              Domain language and decisions
```

Zero code comments, including generated code; contracts, names, fixtures, and ADRs carry explanations. Preserve dependency license obligations. Direct SQL access, if exposed, uses versioned read-only views, not internal tables.

## 6. ADR list

- **One authority, equal replicas:** Apple remains authoritative; Mac sequences bridge state and commands; no CRDT or direct-iCloud adapter in v1.
- **Typed duplex Apple seam:** authenticated SwiftHelper/SimApplePeer substitution contains Apple variability and makes sim E2E meaningful.
- **One transactional State module:** records, journal, checkpoints, and command transitions share atomicity; retention lifetimes remain distinct.
- **Evidence over exactly-once effects:** preserve indeterminate outcomes and admission eras instead of retrying uncertain Apple actions.
- **One visibility policy:** scoped reads and trusted replication share enforcement without claiming control over copied files.
- **Push journal with replacement snapshots:** durable cursor coverage and epoch changes define recovery; no polling replicas or extra broker.
- **Capability evidence controls exposure:** full-depth Messages remains the goal; unsupported operations reject; Notes follows core; Health stays unavailable without a demonstrated Mac source.
- **E2E control tool first:** feature evidence precedes breadth; code-level tests cover only decoders, time math, and unreachable crash/race timing.

## 7. Modularity risks

- **A giant “generic source.”** Share lifecycle only; typed domain operations prevent Calendar recurrence or Messages causality leaking into generic dictionaries.
- **Transactional logic split into shallow repositories.** Keep State deep. By the deletion test, removing it must redistribute atomicity/recovery work, not merely remove delegation.
- **Protocol-shaped depth illusion.** One stream method still has ordering complexity; keep its state machine and conformance scenarios together.
- **Duplicate product semantics.** Frontdoor adapters stay thin; Operations owns meaning, Trust owns visibility, Swift owns native behavior. Each fix has locality and leverage across clients/tests.
- **Fake evidence replacing native proof.** Sim validates reliability, not Apple capabilities. Real tests and the measured Screen Time limitations remain release gates.

Finding coverage: **1** glossary/coverage; **2–3** State/feed; **4–7** Operations/IPC; **8–10** Messages; **11–12** Agenda; **13** Contacts/capture; **14** Blobs/scheduling; **15** Contracts/views; **16–17** Trust/Runtime; **18** Runtime; **19** Control; **20** concrete modules and E2E-first scope.
