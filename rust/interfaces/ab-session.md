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
contacts.containers, contacts.groups, contacts.contacts, contacts.history, ui.target,
ui.actions, focus.status, location.status, mac.state, permissions.status. No generic SQL/script selector.

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

Session owns selection; readers emit observations only (no gateway handle). Per source:
Swift (default), Shadow (Swift authoritative, Rust diff only), Rust (future routed mode).
Proxy rejects any configured authoritative Rust producer; it cannot safely filter IPC.
One external session. In Swift/Shadow mode Swift assigns capture IDs, spends credits,
receives Committed and persists checkpoints. In routed Rust mode ab-session does all four;
ab-native only reads/diffs. Switch only disconnected after all captures committed;
reopen with committed checkpoint, exactly one selected producer. No hot takeover.
Shadow compares key+canonical data+scope+generation+completeness; never sends captures,
spends credits, acknowledges or persists an authoritative checkpoint. Partial scans cannot
certify equality or deletion. Native/1 facts are NOT ipc/1 Captures.

Native request_hash: lowercase SHA256 hex of canonical args JSON (recursively sorted keys, compact UTF-8, unescaped slash, no non-finite numbers). Separate from frozen ipc/1 request_hash. Hello includes positive owner_fence; permit must match it. expires_at is Unix seconds (integer).

Swift integration agreement: Helper reserves separate native socketpair child fd 200 via AB_NATIVE_FD; raw IPC proxy does not read it or send Hello. Native server rejects effects conflict while Swift IPC owns authority. Source adapters initially support full bounded scope args={} only; unsupported filters return unsupported. contacts.history accepts history_token. Scope may be structured {collections,history_token}; pages bind exact scope equality. Release uses args.file_ref and replies Released with id.

Binary selection configuration: AB_SESSION_PRODUCERS JSON object maps source names to
"Swift", "Shadow", "Rust". Default Swift. Rust fails closed in this proxy build.
Reader integration: session-owned adapters wrap ab-native readers and implement ownership::ShadowReader::snapshot(SourceId) -> ObservationSet (ab-native must not depend on ab-session);
call shadow_once with Swift fact adapter and ab-native reader. API accepts no writer or
checkpoint sink and returns Equal/Incomplete/Different. Records are canonical JSON bytes
keyed by native key; no duplicate keys. ab-native adapters remain worker-owned.

## Swift endpoint delivery

The Swift implementation is in NativeProtocol and AppleSession; native/NATIVE-1.md
records launch and activation details. NativeChecks uses fake adapters and the
synthetic Calendar range / Contacts list scenario facts. Scalar scopes are strings;
collection scopes include collections and the Contacts history_token. Contacts history
pages individual changed identifiers, without implying deletion. Completed snapshots
release their retained rows; continuation after terminal completion returns expired.
Existing registered query operations maps.snapshot, pdf.search, audio.transcribe and
shortcuts.actions, plus shortcuts.run, are permit-gated native effects. The closed
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
sending the native permit. Swift's IPC Dispatch entry point rejects calls in
this mode; its compatibility guard uses a separate inactive directory. Swift
validates and consumes its prepared ticket through native/1 and publishes the
resulting captures. Rust correlates only acknowledged capture identities from
frames sent after dispatch started, checks the request fingerprint, and fsyncs
the Outcome before sending it. Missing commits, mismatched native IDs, transport
failure or unavailable evidence remain indeterminate. Resolve returns the saved
outcome without repeating an effect; unresolved guard entries are reported at
Ready after restart. The default Swift path retains its existing resolution.

Framework queries in the native effect catalogue also receive durable permits;
read-only queries continue through IPC. Native query results currently support
inline JSON only. Blob-producing queries remain indeterminate until native file
references and IPC BlobRead ownership are integrated. Query cancellation has no
native cancellation message; the connection read deadline bounds its wait.
Operations outside the existing native effect catalogue cannot use Swift IPC
Dispatch as a fallback. This is an isolated routing gate, not production effect
qualification. Existing capabilities must be narrowed before enabling it for a
production Helper. Authoritative Rust source selection is still rejected;
reader scheduling, capture credit and checkpoint persistence integration remain
separate work.

## Rust capture producer (per source, lab-only today)

Build ab-session with `--features rust-capture`. Without it the binary is unchanged and
`Selection::validate_proxy` still rejects `Producer::Rust`. The mode is separate from the
fd-launched proxy and from `rust-effects`; the two relays are not combined yet.

Entry point: `ab-session --lab-listen PATH --lab-gateway PATH`. It listens where the Helper (or
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
(staging beside the root as `<root>-staging`). `next` drains the hydration queue first (each
hydration is its own flight with an empty scope), then reconciles when the file-tree signature
changed. Generation is `notes-<Z_UUID>` from the store, so a stable store never rescans. The lab
world mirrors `/external-change` for `notes.folder` and `notes.note` into a rebuilt
`NoteStore.sqlite` (`lab/world/session_notes.go`, gzip protobuf bodies, stable Z_PK per note,
revision as Z_OPT). Negative control: `AB_SESSION_RUST_POLL_MS=3600000` makes NOTE-IMPORT fail on
`external_note_imported`. Not mirrored yet: notes effects (`notes.update` and friends),
created/modified dates, pinning dates, deleted or smart folders, html bodies and attachments.


## Production readiness (all switches off by default)

- **Checkpoint persistence.** With a state directory (`AB_SESSION_RUST_STATE` in lab mode, `<home>/session/capture-state` in production) the session fsyncs `<source>.json` (`generation`, `checkpoint`, reader `token`) after the gateway commits a Rust capture and the reader has settled. The token is the committed input-tree signature. On `Open`, a record is trusted only when its generation and checkpoint equal `Open.checkpoints[source]`; the reader then resumes and an unchanged source emits nothing. A mismatch (or no record) means a normal first full scan under the gateway's predecessor, so a stale or crashed state file can only cause a rescan, never a skipped change.
- **Blob handles.** A reader can own a handle prefix (`CaptureReader::owns`/`blob`). Gateway `BlobRead` frames for an owned handle are answered by the session with `BlobChunk` or `BlobUnavailable`, and are never forwarded to the Helper. Voice Memos (`voice_memos`, config `root`, optional `staging`, default `<state>/blobs/voice_memos`) produces `voice-memo-blob:<digest>` handles; the asset store re-adopts staged files on open, so handles keep resolving after a restart.
- **Watcher scheduling.** `ab_sys::watch::Watcher` uses kqueue (`EVFILT_VNODE`, macOS) or inotify (Linux) on every directory and file under a reader's root and re-registers after each event. An event wakes the producer immediately (15 ms debounce). The poll interval is only the fallback; if the watcher cannot start, polling continues unchanged. Lab: `AB_SESSION_RUST_WATCH=1`; production: `watch` in the config file (default true).
- **With `rust-effects`.** `effects::Layer` is an overlay on the same relay (`routed::relay_layered`). Rust captures register their keys with the effects state before being written, so Dispatch evidence correlation sees Rust-committed captures like Swift ones. `effects::relay` now runs through the same relay.
- **Production switch.** `ab-session --gateway-fd N --helper-fd M [--effects-home H] [--capture-home H]`. With `--capture-home`, `<home>/session/rust-capture.json` (`{"sources":{"safari":{"root":"..."}},"watch":true,"poll_ms":5000}`) selects Rust sources (`safari`, `usage`, `notes`, `voice_memos`); an unsupported source fails startup, an absent file or empty `sources` is the verbatim proxy. Flipping a source is editing the file and restarting the Helper; flipping back is removing the entry. The Swift SessionProxy passes `--capture-home` only when the file exists. A binary built without `rust-capture` rejects the flag. Generation changes on a flip cost one rescan and never a false deletion.

Open items: watcher-driven scheduling and persistence are proven for Screen Time in the sim; blob reads and effects plus capture are proven by socketpair tests only (simapple has no native/1 endpoint and no Voice Memos file fixture world); Mail, Notes, Messages and the other sources still need readers.
