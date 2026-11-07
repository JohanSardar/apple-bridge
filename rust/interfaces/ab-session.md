> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# ab-session integration contract

Branch: t3/ab-session-native-proxy. Canonical protocol: design/spec/native-1.md.

## Launch

The Helper connects the gateway socket without authenticating, creates two socketpairs and posix_spawns its bundled `ab-session`
child without disclaiming responsibility (S2):

`ab-session --gateway-fd 198 --helper-fd 199 --session-secret FILE --session-home DIR [--rust-commands LIST]`, with `AB_NATIVE_FACTS_FD=200`.

The child owns the inherited descriptors, and the parent closes its ends and its copy of the gateway descriptor. fd 199 carries the
native/1 session (`design/spec/native-1.md`); fd 200 carries the facts link. EOF or an error ends the session, and the parent reaps the
child before reconnecting. `AB_SESSION_BINARY` overrides the bundled executable in a lab build only; a release Helper verifies the bundled binary's code identity before running it. There is no other mode: the verbatim
proxy, the routed relay, the effects layer and the flags and variables that selected them were removed with the Swift ipc/1 peer.

## Facts link

`AB_NATIVE_FACTS_FD` supplies an inherited connected stream, separate from the session link. Rust client sends {"t":"Hello","protocol":"native/1","version":1,"owner_fence":1};
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

## Reader ownership

ab-session captures every source. The Helper starts no capture; it answers facts and primitives and reports the
readiness of the sources whose readiness is a framework fact. The Helper passes `--user-home DIR` and the session
builds its configuration from that home (`production::defaults`). `<home>/session/rust-capture.json` is an
override: per-source fields merge over the defaults, a source it does not name is still captured, and
`"exclusive": true` limits the session to the named sources (an entry with its own `root` is then taken as
written). Without `--user-home` the file alone decides, and no file means no capture; that is how a fixture run stays
away from the machine's real data.

A `Scan` frame of a source the session captures is refused with `ScanFailed`. `Scan` exists only for the lab's
simulated Helper, for sources the session does not capture in that run.

## Rust capture in session mode

`session/capture.rs` runs the readers of the named sources. Once the session is open a producer thread polls each `CaptureReader` (the
watcher wakes it earlier; the poll is the fallback). A reader returns a pending `Capture`; the session adds `predecessor` (the committed
checkpoint of the same generation, else empty), assigns a checkpoint and scan id, waits for pool credit and writes the frame. One flight per source,
in the same `flight::Flights` as Swift scans. A gateway `Committed` for a reader flight must match checkpoint and credit; it calls
`CaptureReader::commit` while the flight is still open, returns credit and advances the committed checkpoint.

`<source>.reconcile` is answered by the session when `CaptureReader::refresh` returns true (agenda facts readers, Notes, Messages): it returns after a reader pass that began after the request has finished. `CaptureReader::answer` lets a reader serve a command's read from the store it owns (`notes.read`, `notes.observe`, `notes.trash_folder`). `bind` nudges the reader and waits up to 15 s for the capture of a key in an owned source, 0.5 s otherwise.

Readiness: `CaptureReader::readiness` decides state, coverage, capabilities and reason for sources that are read from a store (phone, voicemail, voice memos, usage, mail, notes, messages, files, shortcuts). The session overrides those in `Ready` and sends `SourceState` when one changes, checked every 30 s. A reader that returns `None` (agenda, contacts, Mac, Focus, Safari, PDF) leaves it to the Helper.

Configuration in a lab world: `AB_LAB_SESSION_SOURCES=a,b` writes `session-home/session/rust-capture.json` from fixture directories, and `--lab-session-listen` loads it.

- A scan is split into frames like the Swift sink: 16000 bytes of observations per frame (64000 when
  `live`), absences last, one scan id throughout. Every frame but the last carries a derived checkpoint,
  waits for its own `Committed`, and uses the previous frame's checkpoint as `predecessor`. Only the
  final frame's `Committed` calls `CaptureReader::commit`. Each chunk keeps only handles referenced by its observations. The complete native-encoded frame, including handles, header and derived checkpoint/predecessor, must fit 262144 bytes; the observation targets remain soft so Messages' pre-sized history pages are preserved.
- A reader error or unchanged input yields no frame; absence is never invented. A new connection
  builds fresh readers, so the first scan is full (`live` false) and replays under the gateway's
  committed predecessor; a generation change is one rescan.

Rolling out another source (what to copy, in order):

1. Implement `readers::CaptureReader` for the `ab_native` reader: `next` builds a pending scan
   from `ScanContext` (checkpoint, scan id, now) and returns the serialised batch with
   `coverage` removed; `commit` calls the reader's commit. Skip unchanged input cheaply
   (Safari uses the reader's input signature, Usage a file-tree signature).
2. Register the source name in `readers::from_config` and `production::SUPPORTED`.
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
`CloudRecordings.db`; voicemail prefers `FaceTimeMessageStore-local.sqlitedb` when present. The readers
skip unchanged database/`-wal` input; voicemail also compares current audio versions against the versions captured around staging, including missing paths. Native source races and transient staging errors retry without caching a failed signature. Pending transcripts override unchanged-input skipping until later passes finish enrichment. Arrival/replacement after capture changes the next signature even if it precedes acceptance; acceptance never resamples audio for the committed signature.
Voicemail deliberately declines persisted input tokens and scans once after each restart, rebuilding
asset tracking and refreshing older metadata-only captures even when the database has not changed.
Voice Memos and voicemail serve content-addressed audio through session-owned blob handles. The lab
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


## Readers across restarts

- **Checkpoint persistence.** With a state directory (`<home>/session/capture-state` in production) the session fsyncs `<source>.json` (`generation`, `checkpoint`, reader `token`) after the gateway commits a Rust capture and the reader has settled. The token is the committed input-tree signature. On `Open`, a record is trusted only when its generation and checkpoint equal `Open.checkpoints[source]`; the reader then resumes and an unchanged source emits nothing. A mismatch (or no record) means a normal first full scan under the gateway's predecessor, so a stale or crashed state file can only cause a rescan, never a skipped change.
- **Blob handles.** A reader can own a handle prefix (`CaptureReader::owns`/`blob`). Gateway `BlobRead` frames for an owned handle are answered by the session with `BlobChunk` or `BlobUnavailable`, and are never forwarded to the Helper. Voice Memos and voicemail (config `root` and required `staging`) produce `voice-memo-blob:<digest>` and `voicemail-blob:<digest>` handles respectively; their asset stores re-adopt staged files on open, so handles keep resolving after a restart.
- **Watcher scheduling.** `ab_sys::watch::Watcher` uses kqueue (`EVFILT_VNODE`, macOS) or inotify (Linux) on every directory and file under a reader's root and re-registers after each event. An event wakes the producer immediately (15 ms debounce). The poll interval is only the fallback; if the watcher cannot start, polling continues unchanged. `watch` in the config file (default true).
- **Override file.** `<helper home>/session/rust-capture.json` (the Helper home is `<gateway home>/helper`), for example `{"sources":{"safari":{"root":"..."}},"watch":true,"poll_ms":5000}`. A source outside `production::SUPPORTED` fails startup.

Record ids across a flip. The gateway reuses a record id only for the same `(source, generation, collection, native_key)` or the
same `scoped_alias`, and a Rust reader hashes its own generation, so a flipped source (Phone, Mail, Notes, Messages, Calendar, Contacts)
used to write a second id per item beside the Swift ones and leave the old ones alive; Safari kept its ids only because its aliases carry no
generation. The session therefore adopts the generation the gateway has committed for the source: on its first scan of a connection
the Rust reader's native generation is bound to `Open.checkpoints[source].generation` and every capture reports that generation
(`reported_generation` in `session/capture.rs`; predecessor, flights and the committed checkpoint use the reported one). The binding is saved next to the
checkpoint in `capture-state/<source>.json` (`native`) and restored on `Open` while the gateway's committed generation is still the saved
one. A native generation change while Rust owns the source (a regenerated database) is reported as itself, i.e. a normal generation change.
A reader whose scans legitimately
carry several generations (`CaptureReader::adopts_generation` false: Safari) is not rewritten. `tests/generation_adoption.rs` fails on the
id doubling, on a rollback that duplicates or loses a record, and on a regenerated database being adopted.

Files has no alias and the old Swift capture made a new generation per Helper start, so an existing home holds alive `files.item`
records from past generations. Adoption keeps the ids of the last generation and stops the growth; the older ones stay until someone
removes them on purpose.

Bundling: `AB_SESSION_BINARY=<path>` or `AB_SESSION_BUILD=1` (builds `ab-session` through `packaging/dev/build-session.sh`) makes `packaging/dev/build-helper.sh` copy it to `Contents/MacOS/ab-session`; `sign.sh` signs it with the identity of the app before the app itself, so the app's designated requirement is unchanged. Both values are part of the signed-app cache key.

Staging never lives in an Apple container. `voice_memos`, `voicemail` and `notes` need a `staging` path that is not inside or above the source root (symlinks and `..` resolved); `production::load` fills `<home>/session/staging/<source>` when it is absent and `from_config` fails without one. `tests/staging_confinement.rs` fails if a scan, commit or blob read creates any path under the root. Staged files are copied to a temp name and hard-linked into place inside the staging directory, so no rename crosses a device.

Open items: watcher-driven scheduling and persistence are proven for Screen Time in the sim; blob reads and effects plus capture are proven by socketpair tests only (simapple has no native/1 endpoint and no Voice Memos file fixture world); the other sources still need readers.

### Calendar and Contacts (facts-driven)

`calendar` and `contacts` are sources whose Rust reader is `readers::AgendaReader`. Framework
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
is rebound to the Rust checkpoint that last committed that key (`Hub::bind`). For Calendar and
Contacts the Helper `Committed` is held until the Rust commit, so the binding is immediate and uses the latest
capture of the key even when the command changed nothing; Notes waits for a capture after `Dispatch`. A
`Resolve` after a gateway restart binds the same way. Multi-part Helper scans (`scan_id` set,
`completion_evidence` not `complete`) are merged by the reader; their `Committed` is released at once, the
final part is held, and the diff runs once per scan. The producer is woken by every ingest. The lab listener
keeps accepting while the gateway is restarting.

Fact paging: `{"calendar":{"root":"DIR","facts":"native"}}` makes the reader a native/1 client on the facts link
(`AB_NATIVE_FACTS_FD`), a separate stream with a `NativeServer` that answers `Facts` and `Revalidate` and pushes `Changed`. The descriptor must
differ from the gateway and session descriptors. `ipc/1` and `feed/1` are unchanged.
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
carry the same fields as the IPC capture. `reminders` is also a fact source for outcome binding (`facts_source` in `session/commands.rs`): the latest committed capture of the key binds the Outcome.

Rust owns the baseline and the deletion decision for these three sources (`AgendaReader` with `facts`).
Swift enumerates only what the framework returns now (`CalendarSource.enumerate`, `ContactsSource.enumerate`,
`RemindersSource.enumerate`) and answers `Revalidate` with a framework fact (`revalidate` on the same sources);
it keeps no baseline for this path, and the native facts handler has no `ObservationSink`. After a complete
pull, `AgendaReader` asks `native_facts::Client::revalidate` about the identities the diff cannot account for
(`agenda::Reader::unaccounted`) and passes only the answers to the diff; a record is deleted or evicted only
on `missing` / `outside_scope`. Contacts containers are asked like contacts and groups (`contacts.containers`): a container that a
snapshot omits (a transient framework answer, or an incremental Helper frame that lists only changed rows) stays a candidate and is
tombstoned only on a `missing` verdict; Swift's own scan confirms a container the same way before it emits `absent`. An `Error`, a short or reordered answer, a timeout, a closed stream or a new
generation emits nothing and deletes nothing (`tests/native_revalidation.rs`; simapple control
`AB_LAB_SESSION_REVALIDATE_CONTROL=refuse|partial`). The reader token carries the diff state, so a restart
keeps its baseline (`resume`). `native_facts::Client` multiplexes frames by id on one stream with a reader
thread; `Changed` marks the source due, `facts_poll_ms` is only the fallback.

### Safari live tabs (facts)

`safari.tab` comes from AppleScript. `SafariReader` pulls the `safari.tabs` fact every 5 s on its own thread, so a slow script never holds up the capture loop. The answer is the full current tab set; `safari::Reader::tabs` diffs it against the last committed tabs (upserts for changes, `absent` for closed tabs). A failed read changes nothing.

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
`messages.reconcile` is answered by the reader (`CaptureReader::refresh`).

Isolation: the gateway thread never waits for a scan. The session keeps one
lock per reader, answers BlobRead for a handle owned by a `BlobServer` lock-free and waits at most two
seconds for the reader behind a prefixed handle (`notes-blob:`, `voice-memo-blob:`) before replying
`BlobUnavailable`. `Committed` calls `commit` before the flight is released, so the producer cannot start
the next page of a source before the previous one is applied. The gateway's `read_blob` timeout (10 s) now
fails only that read; it used to shut the Helper connection, which reset every source.

Lifecycle: `ab-session` resets SIGTERM/SIGINT/SIGHUP/SIGQUIT to default and unblocks all signals at start
(a parent that ignores or blocks SIGTERM is inherited by `posix_spawn`; SessionProxy now also passes
`POSIX_SPAWN_SETSIGDEF` and `POSIX_SPAWN_SETSIGMASK`), and exits when its parent pid changes.
Not in the Rust reader: the asynchronous pending-blob state; large attachments are hashed on the poll thread.

Flip on an existing home. A capture upsert replaces the whole record `data`; a new generation reuses the
existing record ids through `scoped_alias` (`<collection>/<native_key>`) and a scan only tombstones what the
reader marks `absent`, so an offloaded file (state `offloaded`, never read because it is dataless) is not a
deletion. What would have been lost is the data the Rust reader does not produce: `blob.digest` of an
attachment that is now offloaded or unreadable, and `media` / `derivatives`. `ab-state`
(`attachments::keep_known`, applied in `commit_capture` to `messages.attachment` only) therefore carries the
previous record's `blob.digest` into an incoming `offloaded` or `missing` blob that has none, and carries
`media` and `derivatives` that the incoming record omits when its blob digest equals the previous one. A
changed digest, a `pending` blob, or a non-empty `media` / `derivatives` in the incoming record replace as before (an empty object counts as omitted: a failed or unreachable `messages.media` answer re-emits `media:{}` / `derivatives:{}` and must not empty what is known), and a
tombstoned record is never revived with old fields. The rule is a no-op for the Swift producer, which always
emits those keys. Derivatives for attachments first seen after a flip are built by `messages_media`: Rust decides which attachment, which digest, when to retry and how the row records the result; Swift answers one narrow `native/1` fact, `messages.media`, with ImageIO / AVFoundation and its digest-keyed cache.

- **Link.** The messages source needs `"facts":"native"` (the inherited native/1 stream, as for `files` and `mac`); without it the reader behaves as before and attachments carry no `media` / `derivatives`. `media_retry_ms` (default 60000) is the retry delay.
- **Request.** Once a `present` attachment has its SHA-256, the reader asks `messages.media` with `{digest, file, uti, mime, sticker, companions}` (`file` and `companions` are absolute paths from the `chat.db` rows, confined by Swift to `<home>/Library/Messages/Attachments`; Swift never opens `chat.db`). One worker thread asks, one attachment at a time, with a ten minute patience that fails only that request: a late answer never closes the shared native/1 stream. The row is emitted first without `media` / `derivatives` (support is not known yet), so the message and the attachment never wait.
- **Answer.** One item: `status` `ready` (with `root`, the Helper's derivative directory, and `result`, Swift's `MediaResult`: `media`, `derivatives` with `file` relative to `root`, optional `unavailable` and `awaitingLive`), `unsupported` (`MediaDerivatives.supports` is false: the row keeps no `media` / `derivatives`, as Swift leaves it), `paused` (thermal or battery pressure: asked again after five seconds, nothing recorded) or `failed`.
- **Row.** `ready` re-emits the attachment with `media` = `result.media` and `derivatives` = each variant without `file` plus `"state":"present"`, then `{"state":"unavailable","reason"}` for each `unavailable` entry, the same shaping as `MediaResult.attachmentFields()` which Swift's own `attachMedia` now calls. `blob_handles` maps each variant digest to `media:<digest>`; Rust serves those through `BlobRead` straight from `root` (relative names without `..`, size checked). `failed` or an unreachable Helper re-emits with `media:{}` and `derivatives:{}` and asks again after the retry delay; `awaitingLive` (a Live Photo whose `.mov` has not arrived) records the partial result and asks again after the delay with up to 16 nearest `.mov` attachments as `companions`. A result whose digest no longer matches the attachment is dropped.
- **Differences from Swift.** Swift yields to foreground activity, hot battery and thermal state inside `make`; Rust asks one at a time and honours only Swift's own `paused` answer. Swift looks at every `.mov` attachment for a Live Photo companion, Rust at the 16 nearest by row id. Swift's poster upgrade of an older cached result is not triggered for new attachments. During the first emission Swift writes `media:{}` before it knows the result; Rust writes nothing until the answer, so the final row is identical and only the placeholder differs.
The sim MSG family cannot run with the session on: simapple's message records are a model (`title`,
synthetic keys) that cannot be inverted into a `chat.db`, so the family stays on the Helper path in the
sim and the reader is proven by `tests/capture_producer.rs` and the 46-stage Swift/Rust projection oracle.


## Session mode (step 1 of the Swift boundary plan)

`ab-session --gateway-fd N --helper-fd M --session-secret FILE --session-home DIR` makes ab-session the only
ipc/1 peer: `session::serve` authenticates, validates `Open`, speaks the native/1 session and command frames of
`design/spec/native-1.md` to Swift and owns credit, flights, chunking, checkpoints, predecessor, `Committed`,
tickets, the dispatch guard (`<DIR>/dispatch-guard.log`), rejection, resolve and evidence-to-checkpoint
binding. `tests/session_peer.rs` drives it over socketpairs and
`tests/session_launch.rs` launches the binary with inherited descriptors.

One flight tracker: `flight::Flights` holds the credit pool, one flight per source, committed checkpoints and the commit-ordered record of written keys.

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
Messages is routed per operation (`messages.send`, `messages.reply`, `messages.react`, `messages.edit`,
`messages.unsend`, `messages.delete_message`, `messages.mark_read`, `messages.mark_unread`,
`messages.delete_chat`) or as the bare source `messages` for all nine; the Helper default list
(`AB_SESSION_RUST_COMMANDS`) does not include it. The Linux lab runs all 34 Messages scenarios on the old path
and with the bare source owned by Rust with identical results (the six that differ from green are the
Darwin-only native checks, which fail the same way on both). `session/messages_selftest.rs` orchestrates the
self-test over Swift primitives; `ab-session --selftest-messages <fd> <request-json>` serves it on an inherited
native/1 socket and the Helper's `selftest messages` launches it. A `Resolve` for an owned operation is answered from the guard log unless the
outcome is open (indeterminate, locally observed, or a failed aggregate with an unsettled part), and the capture
checkpoint of an observed Messages part is bound even while the aggregate is indeterminate, as the Swift
dispatch does.

### Notes media facts and Usage clipping (real-data fixes)

- **Notes hydration stall.** The reader needs three Helper facts while it hydrates a note: `mime:<uti>`, `supports:<uti>|<ext>` and `derivatives:<digest>`. The session supplied only the hashtag mime, so the first note with an image attachment returned `FactsRequired`, was re-queued at the front, and every later `next` retried it forever; bodies behind it stayed `pending`. The reader now re-queues a failed job at the back, and `NotesReader` resolves facts through `media_facts::MediaFacts`, which runs the Helper executable as `AppleBridgeHelper media-facts` (request JSON on stdin, response JSON on stdout, two minute deadline, derivative bytes read from the session-owned `media_root`, default `<home>/session/media-derivatives`, via `AB_MEDIA_DERIVATIVES_ROOT`). Facts come from the same `UTType` and `MediaDerivatives` code the Swift reader uses, so records are byte-identical. A fact that cannot be resolved leaves that note pending, is retried after 60 seconds and never blocks other notes. The executable is the sibling `AppleBridgeHelper` of `ab-session`, or `media_helper` in the source config.
- **Usage.** The session now passes the local `knowledgeC.db` display-lock and lock intervals to the Usage reader (and includes the database and its `-wal` in the change signature) when the root is `~/Library/Biome`, as the Swift source does; the hardware id is `IOPlatformUUID`. Live power-sleep events exist only in the Swift process and are not reproduced.

