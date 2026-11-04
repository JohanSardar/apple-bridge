# ab-session integration contract

Owner worktree: /Users/johansardar/.t3/worktrees/apple-bridge/t3-6e4deed1
Branch: t3/ab-session-native-proxy. Canonical protocol: design/spec/native-1.md.

## Launch / default proxy (Phase 6 step 1)

Helper opens the gateway Unix socket WITHOUT authenticating, creates a local socketpair,
and posix_spawns its bundled ab-session child without disclaiming responsibility (S2).
Child arguments: `--gateway-fd 198 --helper-fd 199`. Both descriptors are connected Unix
streams, distinct, >=3. Child owns inherited descriptors. Parent closes child ends and its
copy of gateway fd. Existing PeerWire uses parent socketpair end in place of connecting;
its existing Challenge/Authenticate handshake and all other IPC logic remain unchanged.
Proxy relays raw bytes including authentication; it does not parse or re-encode frames.
EOF / errors terminate the connection; parent reaps child before reconnect. No daemon,
network listener, secret parsing, capture assignment, or guard in the child proxy path.
Default remains existing direct Swift connection; opt in with AB_SESSION_PROXY=1 and
AB_SESSION_BINARY (lab override; production bundled executable).

## Native endpoint (separate socketpair, inactive in proxy mode)

AB_NATIVE_FD supplies an inherited connected stream. Native/1 MUST NOT share the IPC
proxy stream. Rust client sends {"t":"Hello","protocol":"native/1","version":1,"owner_fence":1};
Swift replies {"t":"Welcome","protocol":"native/1","version":1}.
Frames: u32 big-endian byte length + UTF-8 JSON object, 1..262144 bytes. Unknown t/op,
wrong version or missing required field rejected. Requests correlate by nonempty id.

Fact request: {t:Facts,id,op,args,snapshot:null,cursor:null,limit:100}; subsequent pages
repeat op,args,snapshot and cursor. FactPage: {t:FactPage,id,op,snapshot,items:[],cursor:null,
complete:true,scope,generation}. Snapshot token opaque and pinned; complete true ONLY
on successful terminal exhaustive page, no missing rows, stable scope/generation.
Interrupted/truncated/expired scans never imply absence. Error: {t:Error,id,code}.
Codes: invalid,unsupported,unavailable,permission_missing,expired,conflict,corrupt.
Closed fact ops: agenda.calendars, agenda.events, agenda.reminder_lists, agenda.reminders,
contacts.containers, contacts.groups, contacts.contacts, files.ubiquity, mac.state. No generic SQL/script selector. `Revalidate`
(`id`,`op`,`args.identities`) answers `Revalidated` with one `state` (`present`/`missing`/`outside_scope`)
per identity, and Swift pushes `Changed` (`source`,`generation`); both ride the facts stream.

Effect request: {t:Effect,id,op,args,permit:{command_id,owner_fence,ticket,request_hash,
operation,expires_at,nonce}}. EffectResult: {t:EffectResult,id,evidence:{...}}.
Permits bind op, canonical request hash, fence, ticket, expiry and one-shot nonce.
Swift rejects missing permit, wrong op/fence/hash, expired or reused nonce before effect.
Rust alone mints permit after fsync DispatchStarted, feature `rust-effects` OFF by default.
No automatic fallback between Swift and Rust command authority. Queries which trigger
framework/network work (Vision, Speech, Maps) are effects too. Permission facts never prompt.
File refs: {path,size,sha256,mime}; confined to session-owned 0700 exchange root, regular
0600 files, no symlink traversal; receiver verifies bytes/hash and acknowledges release.

## Reader ownership API

Exactly one producer per source, decided at registration. Per source: Swift (default) or Rust.
When the capture switch (`session/rust-capture.json`) names a source, the Helper does not start
that source's Swift capture at all (`CaptureOwnership` in AppleSession; Safari keeps only its live
tabs); it still registers the source for commands, queries and facts. Proxy rejects any configured
authoritative Rust producer; it cannot safely filter IPC. In Swift mode Swift assigns capture IDs,
spends credits, receives Committed and persists checkpoints. In routed Rust mode ab-session does all
four; ab-native only reads and diffs. Switch only disconnected after all captures committed; reopen
with committed checkpoint. No hot takeover. Native/1 facts are NOT ipc/1 Captures.

Native request_hash: lowercase SHA256 hex of canonical args JSON (recursively sorted keys, compact UTF-8, unescaped slash, no non-finite numbers). Separate from frozen ipc/1 request_hash. Hello includes positive owner_fence; permit must match it. expires_at is Unix seconds (integer).

Swift integration agreement: Helper reserves separate native socketpair child fd 200 via AB_NATIVE_FD; raw IPC proxy does not read it or send Hello. Native server rejects effects conflict while Swift IPC owns authority. Source adapters initially support full bounded scope args={} only; unsupported filters return unsupported. contacts.history accepts history_token. Scope may be structured {collections,history_token}; pages bind exact scope equality. Release uses args.file_ref and replies Released with id.

Binary selection configuration: AB_SESSION_PRODUCERS JSON object maps source names to
"Swift" or "Rust". Default Swift. Rust fails closed in this proxy build.

## Swift endpoint delivery

The Swift implementation is in NativeProtocol and AppleSession; native/NATIVE-1.md
records launch and activation details. NativeChecks uses fake adapters and the
synthetic Calendar range / Contacts list scenario facts. Scalar scopes are strings;
collection scopes include collections and the Contacts history_token. Contacts history
pages individual changed identifiers, without implying deletion. Completed snapshots
release their retained rows; continuation after terminal completion returns expired.
Existing registered query operations maps.snapshot, pdf.search, audio.transcribe and
plus shortcuts.run, are permit-gated native effects (shortcuts.actions is answered by ab-session itself when the shortcuts source is routed: `ab_native::shortcut_actions`). The closed
Swift catalogue is NativeServer.effectOperations. The proxy leaves effects disabled;
AppleSession.runNative is exclusive with the IPC run loop and is not selected by the
current raw Rust proxy. No live framework parity or production cutover was performed.

## Opt-in effect routing

Build ab-session with `--features rust-effects` and select both
`AB_SESSION_PROXY=1` and `AB_SESSION_EFFECTS=1` before starting an isolated
Helper. SessionProxy supplies `--effects-home` using the actual Helper home.
The default remains the verbatim proxy. A binary without the feature rejects
this selection; it never falls back to Swift dispatch. No live selection was
changed by this implementation.

The routed proxy keeps authentication and capture bytes intact, retains Prepare
and Prepared bindings, consumes each ticket once, and binds Dispatch origin,
command, fence and deadline. Rust fsyncs the authoritative dispatch guard before
sending the native permit. Swift's IPC Dispatch entry point rejects calls for
routed operations; the Rust guard lives in `<home>/native-effects` and Swift's
own guard keeps `<home>`. Swift
validates and consumes its prepared ticket through native/1 and publishes the
resulting captures. Rust correlates only acknowledged capture identities from
frames sent after dispatch started, checks the request fingerprint, and fsyncs
the Outcome before sending it. Missing commits, mismatched native IDs, transport
failure or unavailable evidence remain indeterminate. Resolve never repeats an
effect (see below); unresolved guard entries are reported at Ready after
restart. The default Swift path retains its existing resolution.

Framework queries in the native effect catalogue also receive durable permits;
read-only queries (`mail.body`, `files.search`, `files.read`) continue through IPC. A native query
result carries its blobs as `native_query_blobs` (digest, size, handle); the gateway reads them with
IPC `BlobRead`, which the session forwards to the Helper because Rust does not own those handles.
A Swift query failure with code `invalid` or `unsupported` travels as `native_query_error` and becomes
the matching `QueryError`. Inline results are capped by the 262144 byte native frame; native file
references are not wired for queries.

### Effects end to end (opt-in, off by default)

`effects::COMMANDS` plus `effects::EFFECT_QUERIES` is the closed catalogue; it equals
`NativeServer.effectOperations` (a test parses the Swift file and compares the sets). It covers every
command a source advertises: `calendar.*`, `reminders.*`, `contacts.*` (with groups), `messages.*`,
`notes.*`, `mail.*`, `files.*` (including `files.transform`), `pdf.annotate`, `speech.say`,
`capture.*`, `mac.screen_record`, `print.document`, `maps.open`, `shortcuts.run`, `focus.set` and the
`mac.*` commands (including `mac.iwork_export`, which is a command, not a query).

Routing is per source. `AB_SESSION_EFFECTS=1` (with `AB_SESSION_PROXY=1`) routes every catalogue source;
`AB_SESSION_EFFECTS_SOURCES=calendar,mail` names sources (source = `captureRoute` of the operation:
`mac.screen_record` and `print.document` belong to `capture`); in production the same list is
`<home>/session/rust-effects.json` (`{"sources":["calendar"]}`, an empty list routes nothing). The Swift
SessionProxy passes `--effects-home` when either is present. Operations of a routed source that are
outside the catalogue are refused before the Helper sees them (`Rejected unsupported`; Swift also
refuses them in `prepare` and removes them from the capabilities it advertises). Operations of a source
that is not routed pass through verbatim, including `Prepare`, `Dispatch`, `Resolve` and `Query`, and
the Swift dispatch guard keeps authority for them. The Rust guard lives in `<home>/native-effects`.

Evidence mapping. `AppleSession.dispatchNative` returns the source evidence with its own `disposition`
(`locally_observed`, `delivered`, `delivery_failed`, `failed_no_effect`, `indeterminate`); Rust keeps it.
`observed: true` evidence is then verified: `focus.set` needs `readback.authorization`, `mac.*` a readback,
`speech.say` and `capture.play_sound` a finished callback bound to the request hash, capture and file
operations a committed capture of `native_key`. Facts-driven sources (Calendar, Contacts) and sources
Swift captures take the latest committed checkpoint of the key; Rust-owned readers need a capture committed
after the dispatch started. Both wait up to ten seconds for it; a missing capture keeps the evidence and
degrades the outcome to `indeterminate` (`reason: capture_not_observed`). A validation failure on the Swift
side travels as `evidence.rejected {code,message,evidence}`; Rust records it as a durable `Rejected` frame
(`rejected_before_dispatch`) and returns it, with `capture_checkpoint` for conflict evidence of a known key.
`Resolve` for an id in the Rust guard is forwarded to the Helper with the saved outcome as `saved`; Swift
re-observes exactly as it did from its own guard and the resulting `Outcome` is written back to the Rust
guard. Rejected entries are answered directly.

Notes pre-state. While it holds the dispatch slot and before it mints the permit, Rust reads the Notes
store with `ab_native::notes_reader::pre_state` (config `sources.notes.root`, else `AB_SESSION_NOTES_ROOT`,
else the user's Group Container) and adds `rust_pre_state: {native_fingerprint}` to the Effect args, so the
permit's request hash binds it. A missing note is `Rejected conflict`, an unreadable store `unavailable`.
Swift `NotesSource.validate(_:preState:)` compares it with the fingerprint recorded at `Prepare` and no
longer reads the store; `dispatchValidated` skips the second read. `prepare` and the dispatch body still
use `NotesStore` (target alias and verification loop).

Request hash. Numbers must round-trip identically on both sides: unsigned integers pass, negative integers
within 2^53, floats are normalised (`2.0` is `2`), floats below 1e-4 or at least 1e15 are refused.

Lab: `AB_LAB_SESSION=<ab-session> AB_LAB_SESSION_EFFECTS=1` (with `AB_LAB_SESSION_SOURCES` optional) starts
the session with `--lab-native native.sock` and simapple with `--native-socket`; simapple runs each Effect
through its existing Dispatch (or Query) handler on the session loop and answers with the outcome as
evidence. `AB_LAB_SESSION_EFFECTS_CONTROL=refuse` is the negative control (every native effect fails).
`AB_LAB_SESSION_EFFECTS_EVIDENCE=<dir>` keeps each world's guard and process logs.

Not production qualification: no live Helper has used this path, Swift `DispatchGuard` and the IPC
Dispatch entry still exist for unrouted sources, and Resolve after a Helper restart depends on the
Swift source's own re-observation.

## Rust capture producer (per source, lab-only today)

Build ab-session with `--features rust-capture`. Without it the binary is unchanged and
`Selection::validate_proxy` still rejects `Producer::Rust`. The mode is separate from the
fd-launched proxy and from `rust-effects`; the two relays are not combined yet.

Entry point: `ab-session --lab-listen PATH --lab-gateway PATH`, present only in a binary built with `--features lab,rust-capture` (the shipped build has no `--lab-*` flags). It listens where the Helper (or
simapple) would dial the gateway, dials the gateway socket for every accepted connection, and runs
`routed::relay`. Configuration is two environment variables:
`AB_SESSION_PRODUCERS={"safari":"Rust"}` selects sources (others stay Swift/simapple), and
`AB_SESSION_RUST_SOURCES={"safari":{"root":"DIR"},"usage":{"root":"DIR","zone":"UTC","local_name":"..."}}`
gives each Rust source its fixture directory (optional `generation`, default `rust-native/1`).
`AB_SESSION_RUST_POLL_MS` (default 100) is the reader poll interval. Every Rust source must have
a config entry and be in the supported list (`mail`, `notes`, `phone`, `safari`, `usage`, `voice_memos`, `voicemail`); otherwise startup fails.

Behaviour of `routed::relay(gateway, helper, readers, poll)` (framing and byte-credit accounting
copy `effects.rs`: whole frame length including the 4-byte header is the credit):

- Authentication and every frame for non-Rust sources pass through as raw bytes.
- `Open` is parsed: fence, `byte_credit` and `checkpoints` seed a shared pool. Checkpoints seed the
  per-source committed `(generation, checkpoint)` used for `predecessor`.
- A Helper `Capture` for a Rust source is dropped and answered with a synthetic `Committed`
  (same source, checkpoint and whole-frame credit) so the Swift/simapple loop does not stall.
  Other sources' captures register a flight and spend pool credit.
- After `Ready`, a producer thread polls each Rust source's `CaptureReader`. A reader returns a
  pending `Capture` (generation, scan id, scope, `complete` evidence, observations, `live`;
  coverage is removed). Session adds `predecessor` (committed checkpoint of the same generation,
  else empty), assigns a unique checkpoint and scan id, waits for pool credit, writes the frame
  and registers a flight. One flight per source.
- Gateway `Committed` for a Rust flight must match checkpoint and credit; it is consumed, returns
  credit, advances the committed checkpoint and calls `CaptureReader::commit`. It is never sent
  to the Helper. `Committed` for other sources passes through and also updates bookkeeping.
- A scan is split into frames like the Swift sink: 16000 bytes of observations per frame (64000 when
  `live`), absences last, one scan id throughout. Every frame but the last carries a derived checkpoint,
  waits for its own `Committed`, and uses the previous frame's checkpoint as `predecessor`. Only the
  final frame's `Committed` calls `CaptureReader::commit`.
- A reader error or unchanged input yields no frame; absence is never invented. A new connection
  builds fresh readers, so the first scan is full (`live` false) and replays under the gateway's
  committed predecessor; a generation change is one rescan.

Rolling out another source (what to copy, in order):

1. Implement `readers::CaptureReader` for the `ab_native` reader: `next` builds a pending scan
   from `ScanContext` (checkpoint, scan id, now) and returns the serialised batch with
   `coverage` removed; `commit` calls the reader's commit. Skip unchanged input cheaply
   (Safari uses the reader's input signature, Usage a file-tree signature).
2. Register the source name in `readers::from_config` and the `validate_routed` list in `main.rs`.
3. Lab world: `AB_LAB_SESSION=<ab-session>` and `AB_LAB_SESSION_SOURCES=safari,usage,notes` make
   `lab/world/session.go` start ab-session on `session.sock`, point simapple at it, and keep the
   gateway socket unchanged. Fixture roots default to `<world>/session-<source>`; a scenario that
   already owns a fixture calls `world.WithSessionRoot(ctx, source, dir)` (Safari does). If the
   scenario feeds the source through a simapple control route, add a hook in
   `World.ControlAction` that materialises the same data as files (`stageSessionUsage` writes
   SEGB segments and `sync/sync.db` atomically for `/usage`; `stageSessionMail` rewrites an
   `Envelope Index` SQLite file from the accumulated `mail.*` records of every `/dataset`). simapple's own capture for that
   source is dropped, so assertions only see the Rust reader's rows.
4. Prove: the source's sim scenarios must give the same assertions with the world option off and
   on (`--impl gateway=rust`), and a negative control must fail (proves the data comes from the
   Rust reader): Screen Time with `AB_SESSION_RUST_POLL_MS=3600000`, Safari with
   `AB_LAB_SESSION_CONTROL=empty` (the session reads an empty fixture directory), Mail with
   `AB_SESSION_RUST_POLL_MS=3600000`. Safari's
   scenario runs a corruption check over its fixture, so the session reads a pristine copy. Add a socketpair test in
   `tests/capture_producer.rs` for the frame rules.

Calls, Voicemail and Voice Memos (`phone`, `voicemail`, `voice_memos`) follow the same pattern. Their
fixture databases live in the source root as `CallHistory.storedata`, `voicemail.db` and
`CloudRecordings.db`; the reader skips unchanged input by the database and `-wal` signature. Voice Memos
returns no frame when a scan would carry blob handles, because the session cannot serve blobs yet. The lab
world writes these databases in place (`World.StageSessionMedia`) from `verifyNativeMediaGateway` when the
source is in `AB_LAB_SESSION_SOURCES`; the version travels in `duration` (100 + version).
`AB_LAB_SESSION_CONTROL=empty` skips the writes and is the negative control. `AB_SESSION_RUST_POLL_MS` is
not a reliable control when other sources keep committing, because any gate change wakes the poll loop.
Notes: `readers::NotesReader` wraps `ab_native::notes_reader::Reader` over `<root>/NoteStore.sqlite`
(staging in the required `staging` config path, never inside or above the root). `next` drains the hydration queue first (each
hydration is its own flight with an empty scope), then reconciles when the file-tree signature
changed. Generation is `notes-<Z_UUID>` from the store, so a stable store never rescans. The lab
world polls simapple's `/inspect` every 25 ms (and after each `/external-change`) and rebuilds
`NoteStore.sqlite` from its `notes.folder` and `notes.note` records (`lab/world/session_notes.go`):
stable Z_PK per key, Z_OPT bumped when a row changes, created and modified dates, pinning, locked
notes without bodies, offloaded notes without bodies, recently-deleted and smart folders, parent
folders, accounts by identifier, deleted records as `ZMARKEDFORDELETION`, gzip protobuf bodies from
plain text or html (bold, italic, underline, strike, checklist paragraphs, hashtag attachments).
So effects (`notes.create`, `update`, `delete`, `move`, folder create and rename) reach the store
through simapple. simapple skips its expected-state equality check when the session owns Notes (the
gateway's record is Rust-shaped). The session binds a Helper `Outcome` that carries a `native_key`
and `capture_checkpoint` to the next committed Rust capture containing that key (after the
`Dispatch`), rewriting `capture_checkpoint`, so the gateway can observe the effect. The reader
context carries the mime fact for the hashtag attachment type. Negative control:
`AB_SESSION_RUST_POLL_MS=3600000` makes NOTE-IMPORT fail on `external_note_imported`.
Pending bodies: the Rust reader publishes a metadata-only record (`body_status: pending`) before hydration. The Notes source config accepts `hydration_hold` (a path); while that file exists the reader skips draining the hydration queue but still reconciles. The lab world creates it while any simapple note record says `body_status: pending` and removes it afterwards, and writes `native_fingerprint` into the gzip header comment of the body so the store row gets a new revision with the same decoded text. NOTE-CRUD's pending-content steps therefore see the reader's real pending record, then the hydrated one with an unchanged `content_version`. Fixtures that read a note after its first visible version wait for `body_status` to leave `pending` (`notesBodyPending`).
Open: NOTE-LIST asserts a note inside a recently-deleted folder and a fixed thumbnail digest (`bbbb...`). The reader query (`notes_store.rs`, `excluded` CTE: `ZFOLDERTYPE=1` or marked-for-deletion folders and their descendants) never selects a note inside a recently-deleted folder, and a derivative digest is the SHA-256 of staged bytes, so neither can come from a faithful store. With the session on, NOTE-LIST fails 11 assertions (`replica_converged` truth_alias_failed, `content_versions_0/1`, `{rest,mcp}_{0,1}_preview_thumbnail`, `{rest,mcp}_{0,1}_recently_deleted_read_only`); with it off it passes. It tests gateway read-model behaviour, so it runs with the session off.


## Production readiness (all switches off by default)

- **Checkpoint persistence.** With a state directory (`AB_SESSION_RUST_STATE` in lab mode, `<home>/session/capture-state` in production) the session fsyncs `<source>.json` (`generation`, `checkpoint`, reader `token`) after the gateway commits a Rust capture and the reader has settled. The token is the committed input-tree signature. On `Open`, a record is trusted only when its generation and checkpoint equal `Open.checkpoints[source]`; the reader then resumes and an unchanged source emits nothing. A mismatch (or no record) means a normal first full scan under the gateway's predecessor, so a stale or crashed state file can only cause a rescan, never a skipped change.
- **Blob handles.** A reader can own a handle prefix (`CaptureReader::owns`/`blob`). Gateway `BlobRead` frames for an owned handle are answered by the session with `BlobChunk` or `BlobUnavailable`, and are never forwarded to the Helper. Voice Memos (`voice_memos`, config `root` and required `staging`) produces `voice-memo-blob:<digest>` handles; the asset store re-adopts staged files on open, so handles keep resolving after a restart.
- **Watcher scheduling.** `ab_sys::watch::Watcher` uses kqueue (`EVFILT_VNODE`, macOS) or inotify (Linux) on every directory and file under a reader's root and re-registers after each event. An event wakes the producer immediately (15 ms debounce). The poll interval is only the fallback; if the watcher cannot start, polling continues unchanged. Lab: `AB_SESSION_RUST_WATCH=1`; production: `watch` in the config file (default true).
- **With `rust-effects`.** `effects::Layer` is an overlay on the same relay (`routed::relay_layered`). Rust captures register their keys with the effects state before being written, so Dispatch evidence correlation sees Rust-committed captures like Swift ones. `effects::relay` now runs through the same relay.
- **Production switch.** `ab-session --gateway-fd N --helper-fd M [--effects-home H] [--capture-home H]`. With `--capture-home`, `<helper home>/session/rust-capture.json` (the Helper home is `<gateway home>/helper`; `flip-capture.sh --home` takes the gateway home) (`{"sources":{"safari":{"root":"..."}},"watch":true,"poll_ms":5000}`) selects Rust sources (`safari`, `usage`, `notes`, `voice_memos`, `mail`, `phone`, `voicemail`, `calendar`, `contacts`, `messages`; Calendar and Contacts take `{"root":"DIR"}` only as the watcher root because their data arrives as Helper facts); an unsupported source fails startup, an absent file or empty `sources` is the verbatim proxy. Flipping a source is editing the file and restarting the Helper; flipping back is removing the entry. The Swift SessionProxy passes `--capture-home` only when the file exists. A binary built without `rust-capture` rejects the flag. Generation changes on a flip cost one rescan and never a false deletion.

Record ids across a flip. The gateway reuses a record id only for the same `(source, generation, collection, native_key)` or the
same `scoped_alias`, and a Rust reader hashes its own generation, so a flipped source (Phone, Mail, Notes, Messages, Calendar, Contacts)
used to write a second id per item beside the Swift ones and leave the old ones alive; Safari kept its ids only because its aliases carry no
generation. The routed relay therefore adopts the generation the gateway has committed for the source: on its first scan of a connection
the Rust reader's native generation is bound to `Open.checkpoints[source].generation` and every capture reports that generation
(`routed::reported_generation`; predecessor, flights and the committed checkpoint use the reported one). The binding is saved next to the
checkpoint in `capture-state/<source>.json` (`native`) and restored on `Open` while the gateway's committed generation is still the saved
one. A native generation change while Rust owns the source (a regenerated database) is reported as itself, i.e. a normal generation change.
Rolling back is the same in reverse: Swift emits its own generation, which is the committed one, so ids stay. A reader whose scans legitimately
carry several generations (`CaptureReader::adopts_generation` false: Safari) is not rewritten. `tests/generation_adoption.rs` fails on the
id doubling, on a rollback that duplicates or loses a record, and on a regenerated database being adopted. `flip-capture.sh` writes
`"facts":"native"` for `calendar`, `contacts` and `reminders`; without that link the Helper's Swift capture is suppressed by the switch and
the reader has no facts.

Bundling: `AB_SESSION_BINARY=<path>` or `AB_SESSION_BUILD=1` (builds `ab-session` with `rust-capture,rust-effects` through `packaging/dev/build-session.sh`) makes `packaging/dev/build-helper.sh` copy it to `Contents/MacOS/ab-session`; `sign.sh` signs it with the identity of the app before the app itself, so the app's designated requirement is unchanged. Both values are part of the signed-app cache key.

Staging never lives in an Apple container. `voice_memos` and `notes` need a `staging` path that is not inside or above the source root (symlinks and `..` resolved); `production::load` fills `<home>/session/staging/<source>` when it is absent and `from_config` fails without one. `tests/staging_confinement.rs` fails if a scan, commit or blob read creates any path under the root. Staged files are copied to a temp name and hard-linked into place inside the staging directory, so no rename crosses a device.

Open items: watcher-driven scheduling and persistence are proven for Screen Time in the sim; blob reads and effects plus capture are proven by socketpair tests only (simapple has no native/1 endpoint and no Voice Memos file fixture world); the other sources still need readers.

### Calendar and Contacts (facts-driven)

`calendar` and `contacts` are routed sources whose Rust reader is `readers::AgendaReader`. Framework
enumeration stays Swift (EventKit, Contacts). The Helper's `Capture` frame for the source is taken as
the raw fact set (upserts are the complete current records, `absent` and `evict` are revalidation
results). `CaptureReader::ingest` stores it, the relay holds the Helper `Committed`, and the session
poll runs `ab_native::agenda::Reader::prepare` (diff, alias replacement, candidate retention, window
eviction) to build the gateway `Capture` with the session's predecessor, checkpoint and credit. When
the gateway commits that capture the reader commits and the held Helper `Committed` frames are released,
so a Helper `wait` returns only after the gateway has the data. Unchanged facts release immediately and
send nothing. A revalidation result for an identity the reader does not know is ignored. Records missing
without a result remain candidates, never deletions. Lab: `AB_LAB_SESSION_SOURCES=calendar` or
`contacts`; negative control `AB_SESSION_RUST_POLL_MS=3600000`.

Command effects: a Helper `Outcome` or `Rejected` whose evidence carries `native_key` and `capture_checkpoint`
is rebound to the Rust checkpoint that last committed that key (`routed::bind_outcome`). For Calendar and
Contacts the Helper `Committed` is held until the Rust commit, so the binding is immediate and uses the latest
capture of the key even when the command changed nothing; Notes waits for a capture after `Dispatch`. A
`Resolve` after a gateway restart binds the same way. Multi-part Helper scans (`scan_id` set,
`completion_evidence` not `complete`) are merged by the reader; their `Committed` is released at once, the
final part is held, and the diff runs once per scan. The producer is woken by every ingest. The lab listener
keeps accepting while the gateway is restarting.

Fact paging (`facts` in the source entry, off by default): `{"calendar":{"root":"DIR","facts":"native"}}` makes the reader a
native/1 client instead of a consumer of the Helper `Capture` frame. `facts: "native"` uses its own inherited stream,
`AB_NATIVE_FACTS_FD` (fd 201), so it runs together with `AB_SESSION_EFFECTS`: the Swift native server handles one request at a
time per stream, and an `Effect` in flight (a transcription, a screen recording) would block every page pull if both shared fd 200,
so effects keep fd 200 (`AB_NATIVE_FD`) and facts get a second stream and a second `NativeServer(factsOnly: true)`, which answers
`Facts` and refuses `Effect` and `Release` with `unsupported` (permit nonces are per server, so effects can never be replayed there).
Without `AB_SESSION_EFFECTS` and without `AB_NATIVE_FACTS_FD` the facts client falls back to `AB_NATIVE_FD`. The two descriptors must
differ from each other, the gateway fd and the Helper fd. `ipc/1`, `feed/1` and the native/1 frames are unchanged.
Files and Mac are Rust captures built on facts (`"facts": "native"` or a socket in the switch file). `ab_native::files` walks the Files roots
itself (the configured root, `roots`, and `Library/Application Support/AppleBridge/files-roots.json` under `home`; symlinks never followed,
10000 items, depth 16, tags from the `com.apple.metadata:_kMDItemUserTags` attribute) and asks Swift only for `files.ubiquity`, 500 paths per
request. The diff is `capture::State` per root: a complete walk reports deletions, an incomplete one (truncated, unreadable subtree) emits
upserts with empty `completion_evidence` and no absence. `ab_native::mac` takes the `mac.state` rows, derives `scoped_alias`, and `capture::State`
decides upserts and absences; Swift pushes `Changed` (`mac`) and ab-session pulls, so the Swift Helper keeps only the framework reads
and the change listeners. Generation for both is `native/1`.
`facts: {"socket": "/abs/path"}` dials a native/1 server (lab: the second connection to the same simapple socket). `facts_poll_ms` (default
15000) is the fallback pull interval. The client (`native_facts::Client`) sends `Hello` with the `Open` fence, then pages
`agenda.calendars` + `agenda.events` (Calendar) or `contacts.containers` + `contacts.groups` + `contacts.contacts` (Contacts) with
`args: {}`, `limit` 500 and the pinned `snapshot`/`cursor`. Every page must echo `id` and `op` and carry the same snapshot, scope
and generation; a non-terminal page must have a cursor `<snapshot>:<n>` and `complete: false`. An `Error` frame, a broken page,
a generation that differs between operations or a terminal page with `complete: false` produces no frame and no deletion; the
next pull starts a new snapshot. Items are the observation rows the Swift scan builds (`upsert`, plus `absent`/`evict` revalidation
results, which `NativeFactSink` now keeps); `agenda::Reader::prepare` does the diff exactly as for Helper facts. A Helper
`Capture` for the source is only a change hint: its `Committed` is held until the pulled capture is committed (or nothing
changed). The first pull is `live: false`.

Reminders (`"reminders"` in `production::SUPPORTED`, off by default) pull the same way: `agenda.reminder_lists` + `agenda.reminders`
through `AgendaReader::reminders` and `ab_native::agenda::Reader::reminders()` (scope `reminders.list`, `reminders.reminder`). The Rust
diff compares `data` and `scoped_alias`, drops a record whose alias reappears under another key in the same collection (the Swift
`tracked` rule), and deletes only on an `absent` row (Swift's `calendarItem(withIdentifier:)` check decides that); an identity that merely
disappears from the page stays. An empty alias is omitted from the capture, as the Swift list rows omit it. The Swift fact scan now emits
`absent` rows for the baseline identities it can no longer find (it did so only for the live scan before), and `AppleSession.facts`
runs the SQLite store enrichment (`store_*` fields) over the pulled rows with the fact scan's own `RemindersStoreEnrichment`, so pulled rows
carry the same fields as the IPC capture. `reminders` is also a fact source for outcome binding (`effects::FACT_SOURCES`,
`routed::holds_facts`): the latest committed capture of the key binds the Outcome.

Rust owns the baseline and the deletion decision for these three sources (`AgendaReader` with `facts`).
Swift enumerates only what the framework returns now (`CalendarSource.enumerate`, `ContactsSource.enumerate`,
`RemindersSource.enumerate`) and answers `Revalidate` with a framework fact (`revalidate` on the same sources);
it keeps no baseline for this path, and the native facts handler has no `ObservationSink`. After a complete
pull, `AgendaReader` asks `native_facts::Client::revalidate` about the identities the diff cannot account for
(`agenda::Reader::unaccounted`) and passes only the answers to the diff; a record is deleted or evicted only
on `missing` / `outside_scope`. An `Error`, a short or reordered answer, a timeout, a closed stream or a new
generation emits nothing and deletes nothing (`tests/native_revalidation.rs`; simapple control
`AB_LAB_SESSION_REVALIDATE_CONTROL=refuse|partial`). The reader token carries the diff state, so a restart
keeps its baseline (`resume`). `native_facts::Client` multiplexes frames by id on one stream with a reader
thread; `Changed` marks the source due, `facts_poll_ms` is only the fallback.

Known gaps: Swift still runs its ipc/1 capture scans (with their baselines) for these sources until the
Helper capture path is deleted; effects keep their own stream (fd 200) next to the facts stream (fd 201)
because `effects.rs` still has its own synchronous native link. The Reminders SQLite store projection and
the Calendar store enrichment are still read in Swift (`RemindersStore*`, `CalendarStoreEnrichment`).

### Safari live tabs (facts)

`safari.tab` comes from AppleScript, so it stays in Swift and arrives as facts. With `safari` in
`session/rust-capture.json` the registration builds `SafariSource(tabsOnly: true)`: only the 5 s tab
loop runs, and its `Capture` (scope `["safari.tab"]`, complete evidence, upserts for every open tab) is
the full current tab set; the first emission is never skipped, even when empty. `SafariReader::ingest`
stores it, the Helper `Committed` is released at once, and `safari::Reader::tabs` diffs it against the
last committed tabs (upserts for changes, `absent` for closed tabs, `live` after the first scan). Frames
with another scope, partial evidence or a malformed row are ignored. History, bookmarks and cloud tabs
are read by the Rust reader from the Safari directory.

### Messages

`readers::MessagesReader` wraps `ab_native::messages::Reader` (`root` is the directory holding `chat.db`;
`home` is the user home whose `Library/Messages/Attachments` and pinning preferences it reads, derived
from a `.../Library/Messages` root when absent). Only the `chat.db`, `-wal` and `-shm` files are watched
(`CaptureReader::watch_paths`), never the Attachments tree.

Paging: `next` never drains the history. A fill runs `Reader::step` until about 128 observations are
queued (or the scan finishes, 8 empty steps pass or 750 ms elapse), then each `next` returns one capture
sized to a single Capture frame (16000 bytes of observations, 64000 while `live`), so every page is one
checkpoint, one Committed and one persisted record. Only the capture that empties the queue after a
completed reconciliation carries `complete` evidence, and every `absent` row travels in it. The
reconciliation scan survives data changes (it used to restart from row 0 whenever Messages wrote to
`chat.db`); a failed page rolls back to its start cursor. `CaptureReader::more` tells the producer to call
`next` again at once, so a long scan neither waits a poll interval per page nor starves other sources
(each loop iteration gives every source one capture).

Durable progress: the committed token is `{generation, cursor, settled_ms}`, saved with the gateway's
committed checkpoint after each Rust capture (`persist` now also saves when only the checkpoint moved).
On `Open` a record is used only when its checkpoint equals the gateway's; the reader then resumes the scan
at `cursor`, or, when the last scan finished less than 15 minutes ago, skips the initial reconciliation and
keeps the 15 minute cadence. A different `chat.db` generation, a stale record or no record means a full
scan. Rows at or before the resume cursor are re-read by the next reconciliation (at most 15 minutes after
the resumed scan ends).

Attachment digests are SHA-256 over the file read through `Reader::read_blob`, cached per guid and declared
size. `blob_handles` maps digest to the attachment guid. BlobRead for a known guid is answered by the
session from a dedicated `chat.db` connection (`readers::BlobServer`), without touching the scan lock.
A Helper `Capture` for `messages` (the Swift `captureCommand` evidence) is not forwarded: its
`(collection, native_key)` list invalidates those fingerprints (`Reader::invalidate`) so the next poll
re-emits them even when the row did not change, and `bind_outcome` can attach the Rust checkpoint.
With `messages` in the config the Helper builds `Messages(rustCapture: true)`: its capture loop idles,
commands and `captureCommand` keep working.

Isolation: the upstream thread (gateway frames to the Helper) never waits for a scan. `routed` keeps one
lock per reader, answers BlobRead for a handle owned by a `BlobServer` lock-free and waits at most two
seconds for the reader behind a prefixed handle (`notes-blob:`, `voice-memo-blob:`) before replying
`BlobUnavailable`. `Committed` calls `commit` before the flight is released, so the producer cannot start
the next page of a source before the previous one is applied. The gateway's `read_blob` timeout (10 s) now
fails only that read; it used to shut the Helper connection, which reset every source.

Lifecycle: `ab-session` resets SIGTERM/SIGINT/SIGHUP/SIGQUIT to default and unblocks all signals at start
(a parent that ignores or blocks SIGTERM is inherited by `posix_spawn`; SessionProxy now also passes
`POSIX_SPAWN_SETSIGDEF` and `POSIX_SPAWN_SETSIGMASK`), and exits when its parent pid changes.
Not in the Rust reader: media derivatives (thumbnails and posters, `data.media` / `data.derivatives`)
and the asynchronous pending-blob state; large attachments are hashed on the poll thread.
The sim MSG family cannot run with the session on: simapple's message records are a model (`title`,
synthetic keys) that cannot be inverted into a `chat.db`, so the family stays on the Helper path in the
sim and the reader is proven by `tests/capture_producer.rs` and the 46-stage Swift/Rust projection oracle.


## Session mode (step 1 of the Swift boundary plan)

`ab-session --gateway-fd N --helper-fd M --session-secret FILE --session-home DIR` makes ab-session the only
ipc/1 peer: `session::serve` authenticates, validates `Open`, speaks the native/1 session and command frames of
`design/spec/native-1.md` to Swift and owns credit, flights, chunking, checkpoints, predecessor, `Committed`,
tickets, the dispatch guard (`<DIR>/dispatch-guard.log`), rejection, resolve and evidence-to-checkpoint
binding. It is exclusive with `--effects-home` and `--capture-home`. The default remains the verbatim proxy and
the routed relay, which keep the Swift ipc/1 peer. `tests/session_peer.rs` drives it over socketpairs and
`tests/session_launch.rs` launches the binary with inherited descriptors.

Not done: no Swift or lab peer speaks these frames yet (Swift `NativeWire` has the framing but no session client,
and `lab-simapple` still dials the gateway). The routed capture producers (`readers`, `production`) are not
reachable from session mode; they stay on the routed relay until a source is moved.

One flight tracker: `flight::Flights` holds the credit pool, one flight per source, committed checkpoints
and the commit-ordered record of written keys. `routed::Gate` owns the only instance in a relay and
implements `flight::Ledger`; `effects::Layer` no longer registers captures or `Committed` itself and
correlates dispatch evidence through the ledger (`sequence()` counts commits, `written()` waits for the
key). Only rows with kind `upsert` or `absent` are correlated.


### Rust-owned commands (step 3, off by default)

`--rust-commands reminders` (session mode; the sources are `session::RUST_COMMAND_SOURCES`) routes the
Reminders `Prepare`/`Dispatch` of the gateway through `session/reminders.rs` instead of the Swift source. The
decisions (operation and writability checks, expected-state conflict, list move, drift between prepare and
dispatch, read-back verification, conflict evidence) are Rust; Swift answers the native/1 `Primitive` frames
documented in `design/spec/native-1.md` and carries no `rust_pre_state`, no ipc/1 command and no ticket. A
source not named keeps the Swift path, and the flag without session mode is refused. Proof today is the
Rust decision tests plus `tests/session_peer.rs` (a fake Swift answering primitives; each rule has a
negative control). Swift answers the primitives (`RemindersSource.primitive`, `AppleSession.nativePrimitive`) and lab-simapple
answers them too (`AB_LAB_RUST_COMMANDS=reminders` starts the lab session with the flag;
`AB_LAB_PRIMITIVE_CONTROL=refuse` is the negative control that fails every `reminders.write`). In native session
mode the Helper always passes `--rust-commands reminders`. `Resolve` after a restart is decided in Rust as well.
Nothing is flipped: native session mode itself is opt-in (`AB_SESSION_NATIVE=1`).

The routed effects layer now refuses a command whose numbers the permit hash cannot represent (below 1e-4 or
at or above 1e15) with `Rejected invalid` (`QueryError invalid` for queries) instead of closing the gateway
stream; the session-mode path never hashes command args.

### Notes media facts and Usage clipping (real-data fixes)

- **Notes hydration stall.** The reader needs three Helper facts while it hydrates a note: `mime:<uti>`, `supports:<uti>|<ext>` and `derivatives:<digest>`. The session supplied only the hashtag mime, so the first note with an image attachment returned `FactsRequired`, was re-queued at the front, and every later `next` retried it forever; bodies behind it stayed `pending`. The reader now re-queues a failed job at the back, and `NotesReader` resolves facts through `media_facts::MediaFacts`, which runs the Helper executable as `AppleBridgeHelper media-facts` (request JSON on stdin, response JSON on stdout, two minute deadline, derivative bytes read from the session-owned `media_root`, default `<home>/session/media-derivatives`, via `AB_MEDIA_DERIVATIVES_ROOT`). Facts come from the same `UTType` and `MediaDerivatives` code the Swift reader uses, so records are byte-identical. A fact that cannot be resolved leaves that note pending, is retried after 60 seconds and never blocks other notes. The executable is the sibling `AppleBridgeHelper` of `ab-session`, or `media_helper` in the source config.
- **Usage.** The session now passes the local `knowledgeC.db` display-lock and lock intervals to the Usage reader (and includes the database and its `-wal` in the change signature) when the root is `~/Library/Biome`, as the Swift source does; the hardware id is `IOPlatformUUID`. Live power-sleep events exist only in the Swift process and are not reproduced.

