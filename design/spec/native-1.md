# native/1 version 1

This private, network-free protocol connects the Helper's Rust child to its signed
Swift framework host over a distinct inherited AF_UNIX SOCK_STREAM socketpair.
It does not replace or change ipc/1. The Helper launches the child using posix_spawn
without responsibility disclaiming, as qualified by rust/SPIKES.md S2. The child
never launches as an independent agent. Swift owns TCC identity and framework calls.

## Transport and handshake

Each frame is a four-byte unsigned big-endian length followed by exactly that many
UTF-8 JSON bytes. Length must be 1..262144 inclusive. Truncated headers/bodies, invalid
JSON, duplicate envelope keys and unexpected message ordering terminate the connection.
Only objects are accepted. One writer serializes frames per endpoint. In-flight requests
have distinct nonempty string IDs. EOF cancels pending work; effects already started
remain indeterminate until evidence resolves them. Reconnection never replays effects.

Rust sends `{"t":"Hello","protocol":"native/1","version":1,"owner_fence":1}`;
Swift responds `{"t":"Welcome","protocol":"native/1","version":1}`. No other frame
is valid before this exchange. Version mismatch fails closed. Descriptors convey
trust within the Helper's process tree; native/1 does not accept network peers.
Each connection has one positive owner fence. Changing fence requires reconnecting.

## Closed fact catalogue

Facts return native observations, never gateway records, capture IDs or committed
checkpoints. They cannot run arbitrary SQL, AppleScript, selectors or shell commands.
Unknown operations return unsupported, not a dynamically discovered method.

| Operation | Arguments / facts | Swift implementation origin |
| --- | --- | --- |
| agenda.calendars | calendar descriptors | CalendarSource/CalendarSource.swift, AgendaGate/AgendaGate.swift |
| agenda.events | calendar IDs, bounded start/end; native events | CalendarSource/CalendarSource.swift, CalendarScanScope.swift, CalendarEventStamp.swift |
| agenda.reminder_lists | list descriptors | RemindersSource/RemindersSource.swift |
| agenda.reminders | list IDs; native reminders | RemindersSource/RemindersSource.swift, RemindersStore/StoreProjection.swift |
| contacts.containers | native container IDs | ContactsSource/ContactsSource.swift |
| contacts.groups | container IDs; group descriptors | ContactsSource/ContactsSource.swift |
| contacts.contacts | container/group IDs; native contact fields | ContactsSource/ContactsSource.swift |
| files.ubiquity | `args.paths`: up to 500 absolute paths under the Files roots; one `{path,status}` per path, `status` is the iCloud downloading status or `local` | FilesSource/FilesSource.swift |
| mac.state | `args` empty; the current `mac.audio_device`, `mac.display`, `mac.power`, `mac.network` and `mac.app` rows (`kind`, `collection`, `native_key`, `data`); `Changed` source `mac` when any of them moves | MacSource/MacSource.swift |

`Facts` fields: `id`, `op`, `args` object, `snapshot` (null on first page), `cursor`
(null on first page), `limit` (1..1000, default 100). `FactPage` fields: `id`, `op`,
`snapshot` nonempty opaque token, `items` array, `cursor` (next token or null),
`complete` boolean, `scope` nonempty string or structured object (including collections and history_token) and `generation` nonempty string.
Source adapters accept args={} for their full bounded source scope; unsupported nonempty filters return unsupported, never false completeness. Enumeration lists only records the framework returns now: no `absent` or `evict` rows, no baseline, no deletion decision.
All pages of a snapshot pin one materialized read, scope, generation and argument
set. Continuations bind operation, args and snapshot; another request cannot reuse
a cursor. No page is larger than the frame limit; reduce count or use file refs.

`complete:true` means the terminal page exhausts the full requested scope, with no
failed, missing, timed-out or truncated reads. Nonterminal pages have complete false.
Truncated terminal pages have cursor null and complete false. Expired snapshots return
expired, never a replacement token. Invalidated Contacts history requires a new full
snapshot. Token advancement happens only after gateway Committed for the corresponding
capture. Notifications invalidate snapshots; they do not imply deletions. `Changed` (below) is an invalidation hint only.
Absence is evidence only after all pages complete with unchanged scope/generation,
as required by SourceSupport/CompleteScan.swift and SQLiteSnapshot.swift.

## Revalidation and push

One server handles frames by `id` concurrently on each stream: a `Revalidate`, a page and an
`Effect` in flight at once never queue behind each other, and a request id already in flight
returns `conflict`. Revalidation and `Changed` ride the same stream as facts; there is no extra socket.

`Revalidate` fields: `id`, `op`, `args:{identities:[{collection,native_key,data?}]}` with 1..500
identities, every `collection` the one `op` enumerates (`agenda.calendars`/`agenda.events`/
`agenda.reminder_lists`/`agenda.reminders`/`contacts.groups`/`contacts.contacts`; containers are never
asked). `Revalidated` fields: `id`, `op`, `results:[{collection,native_key,state}]`, one result per
identity in request order. `state` is a framework fact: `present` (the framework still has it),
`missing` (the framework answered and the record does not exist), `outside_scope` (Calendar only: the
event exists or aged out of the enumerated window). A result count or order that differs from the request,
an `Error`, a timeout or a closed stream is a failed revalidation, never a short answer.

Rust owns the diff and the decision to delete. It asks only about records it knows and a full
enumeration no longer lists, only for an unchanged generation, and it deletes (`absent`) or evicts
(`evict`) only on a well-formed complete answer of `missing` / `outside_scope`. Anything else keeps the
record and emits nothing for it.

`Changed` is pushed unsolicited by Swift: `{"t":"Changed","source":"calendar|contacts|reminders",
"generation":"..."}`. It has no `id`, is an invalidation hint only and never implies a deletion; Rust
pulls on it instead of on a timer.

## Permit-gated effect catalogue

Framework queries that do work, touch devices, make network requests or create files
are effects even where today's Source API calls them queries. Readiness is not proof
an effect is supported: retain existing capability qualification and validation.

| Family | Closed operations | Source origin |
| --- | --- | --- |
| Agenda | calendar.create_event, calendar.update_event, calendar.delete_event, calendar.create_calendar; reminders.create, reminders.update, reminders.complete, reminders.uncomplete, reminders.delete, reminders.create_list, reminders.rename_list, reminders.delete_list | CalendarSource/CalendarSource.swift; RemindersSource/RemindersSource.swift |
| Contacts | contacts.create, contacts.update, contacts.delete, contacts.create_group, contacts.rename_group, contacts.delete_group, contacts.add_member, contacts.remove_member | ContactsSource/ContactsSource.swift |
| UI actor | messages.reply, messages.react, messages.edit, messages.unsend, messages.mark_read, messages.mark_unread, messages.delete_message, messages.delete_chat | MessagesSource/MessagesCommands.swift, MessagesUISession.swift, MessagesPins.swift |
| Automation | messages.send to an exact chat or handle | MessagesSource/MessagesCommands.swift, MessagesRecipient.swift |
| Media | media.derive | MediaDerivatives/MediaDerivatives.swift, Images.swift, Audiovisual.swift, Posters.swift |
| Vision/PDF | vision.ocr, pdf.text, pdf.annotate | VisionSource/VisionQueries.swift; PDFSource/PDFQueries.swift, PDFSource.swift |
| Speech/audio | speech.synthesize, speech.say, capture.play_sound | SpeechSource/SpeechQueries.swift, SpeechPlayback.swift; CaptureSource/CapturePlayback.swift |
| Maps/Location | maps.search, maps.nearby, maps.directions, maps.eta, maps.open, location.current, location.geocode, location.reverse_geocode | MapsSource/MapsQueries.swift, MapsCommands.swift |
| Focus | focus.set | FocusSource/FocusSource.swift |
| Capture/print | capture.screenshot, capture.camera_photo, capture.record_audio, mac.screen_record, print.document | CaptureSource/CaptureSource.swift, CaptureVideo.swift |
| Mac controls | mac.set_volume, mac.set_mute, mac.set_appearance, mac.launch_app, mac.activate_app, mac.quit_app, mac.keep_awake, mac.clipboard_read, mac.clipboard_write, mac.notify, mac.open_settings, mac.set_wallpaper, mac.network_check, mac.bluetooth_devices, mac.time_machine_status, mac.recognize_song, mac.iwork_export | MacSource/MacSource.swift, MacExtrasQueries.swift, MacNetworkQueries.swift, MacClipboardQueries.swift, MacIWork.swift |

These are wire adapters around existing implementations, not new capabilities.
Unsupported adapter operations return unsupported. mac.lock_screen, mac.sleep_display,
mac.restart and mac.shutdown remain unsupported, matching the current prepare rejection.
No permission request operation exists, and no fact operation prompts.

`Effect` fields: `id`, `op`, `args`, `permit`. `EffectResult` fields: `id`, `evidence`.
Permit fields: `command_id`, positive `owner_fence`, `ticket`, `request_hash`, `operation`,
`expires_at` (integer Unix seconds), `nonce` (64 lowercase hex chars from OS randomness).
Hash is lowercase SHA-256 of compact UTF-8 canonical JSON args: recursively sorted object
keys, JSON strings with unescaped slash, finite numbers only. Numeric inputs must use
integers within exact JSON range or strings for precision; no alternate numeric spellings
in permit-bound args. This native hash is separate from the frozen ipc/1 command hash.
Operation, args hash, connection fence, deadline and nonce must all match. Swift consumes
the nonce before executing and rejects reuse, including after failure. Restarted sessions
cannot reuse permits from an old connection. Rust never resends a permit after ambiguity.

Before effects, the UI actor verifies the target and aborts on interference; Agenda and
Contacts re-fetch/validate then write/read back. Evidence retains before/after native facts,
request fingerprint and candidate IDs. Existing postcondition checks remain authoritative.
No arbitrary Automation script or unqualified UI action is accepted.

Rust mints EffectPermit only after fsync of DispatchStarted in dispatch-guard.log.
That record also reserves the command ID permanently; a crash without Outcome recovers
indeterminate and cannot dispatch again. Outcome is fsynced before reporting success.
Tickets are one-shot, bound to origin, fence and expiry by command authority. The guard's
API assumes prepare/validate already succeeded. The `rust-effects` Cargo feature exposes
this API but does not activate a second authority. Swift retains the guard and dispatch
in the default proxy. Authority cutover requires a separate reviewed routing change.

## Files and errors

Large data uses `{path,size,sha256,mime}` file refs rather than base64. Paths resolve
beneath a per-session 0700 exchange directory; files are regular, owned, mode 0600,
not symlinks or hardlinks. Reject traversal and race-prone reopen; validate via opened
handles. Check size and digest before use. `Release` carries request `id` and `args:{file_ref:{path,size,sha256,mime}}`;
`Released` echoes id after consumption before producer cleanup. Disconnect cleans the session
root after in-flight work ends. No arbitrary filesystem paths accepted from native clients.

`Error` fields: `id`, `code`; codes are invalid, unsupported, unavailable,
permission_missing, expired, conflict, corrupt. Failures after dispatch are evidence
of indeterminate outcome, never evidence of safe retry.

## Session frames (ab-session owns ipc/1)

In session mode the Helper starts `ab-session --gateway-fd N --helper-fd M --session-secret FILE`. The
gateway descriptor is an unauthenticated connected stream; the helper descriptor carries native/1 only
and is the only stream Swift uses for gateway traffic. ab-session reads the 32-byte secret (regular file,
mode 0600, parent 0700, same owner), performs the ipc/1 Challenge/Authenticate exchange, validates `Open`
(`ipc/1`, positive fence, `byte_credit` 1..4194304), then runs the native/1 handshake with the gateway's
fence. The ipc/1 byte format toward the gateway is unchanged; frames are built from `ab_wire::IpcFrame` and
`Capture`, and the codecs below live in `ab_wire::native`.

| Frame | Direction | Fields |
| --- | --- | --- |
| `Session` | Rust to Swift, after `Welcome` | `checkpoints`: source to `{generation, checkpoint, refresh_blobs}` from `Open` |
| `Ready` | Swift to Rust, once | `coverage`, `capabilities`, `helper_build`, `os_build`; forwarded as ipc/1 `Ready` |
| `SourceState` | Swift to Rust | `source`, `state`, `coverage`, `capabilities`, `reason`; forwarded as ipc/1 `SourceState` |
| `Scan` | Swift to Rust | `id`, `source`, `generation`, `observations`, `scan_id`, `scope`, `scan_started_at`, `completion_evidence`, `blob_handles`, `live` |
| `ScanCommitted` | Rust to Swift | `id`, `source`, `checkpoint`: the gateway committed the scan's final frame |
| `ScanFailed` | Rust to Swift | `id`, `code`: the scan was not sent (`invalid`) or the session cannot send it (`unavailable`) |
| `Close` | either | ends the session |

Swift sends observations only. ab-session owns the byte credit (a whole frame including its 4-byte header
is the cost; one pool for all sources), one flight per source, chunking (16000 bytes of observations per
frame, 64000 when `live`, 240000 for live Calendar, absences last), checkpoints (a fresh session checkpoint
per scan, derived `-N` checkpoints for intermediate frames), predecessor (the committed checkpoint of the
same generation, else empty) and acknowledgement. A `Committed` that does not match the flight's
checkpoint and whole-frame credit, or that has no flight, closes the session. `Ping` is answered by
ab-session.

### Command frames

| Frame | Direction | Fields |
| --- | --- | --- |
| `Prepare` | Rust to Swift | `command_id`, `principal`, `request_hash`, `origin`, `expires_at`, `action`, `expected_state` |
| `Prepared` | Swift to Rust | `command_id`, `native_state`, `route`, `candidate_ids`, `fingerprint` (the source's own) |
| `Rejected` | Swift to Rust | `command_id`, `code`, `message`, `evidence` |
| `Dispatch` | Rust to Swift | `command_id`, `request_hash`, `action`, `native_state`, `fingerprint` |
| `Outcome` | Swift to Rust | `command_id`, `disposition`, `evidence` |
| `Resolve` | Rust to Swift | `command_id`, `saved` (the guard's outcome) |
| `Query` / `QueryCancel` | Rust to Swift | `id`, `op`, `args`, `deadline_ms` / `id` |
| `QueryResult` / `QueryError` | Swift to Rust | `id`, `result_json`, `blobs` / `id`, `code` |
| `BlobRead` | Rust to Swift | `handle`, `offset`, `length` |
| `BlobChunk` / `BlobUnavailable` | Swift to Rust | `handle`, `offset`, `bytes`, `digest`, `eof` / `handle`, `reason` |

The fields are the ones the gateway already sends and expects; nothing is redesigned here. Authority moves
to ab-session: it validates `Prepare` (non-empty ids, future expiry, one command in flight, no ticket
outstanding, id never seen by the guard), mints the ticket after Swift's `Prepared` (30 seconds or the
command's expiry, one shot), validates `Dispatch` (ticket, origin, command id, fence, expiry), fsyncs
`DispatchStarted` in `<home>/dispatch-guard.log` before it sends `Dispatch` to Swift, writes `Outcome` or
`Rejected` to the same log, answers `Resolve` from the log (offering every recorded outcome except
`failed_no_effect` to Swift with `saved`; Swift decides whether to re-observe or echo it), and reports unresolved dispatches after `Ready` on the next connection. A `Rejected` that
arrives for a started dispatch is recorded as a rejection; a started dispatch without an outcome stays
indeterminate. For an observed `Outcome` with a `native_key`, ab-session waits (15 seconds) for a committed
scan containing that key after the dispatch started (facts sources: the latest) and writes it as
`capture_checkpoint`; otherwise the outcome degrades to indeterminate with `reason: capture_not_observed`.
`evidence.prepared` is filled by ab-session when Swift omits it. Results larger than a native frame fail as
`QueryError unavailable`; the oversized-body path stays a blob handle served through `BlobRead`.

### Primitives (Rust-owned commands, off by default)

`ab-session --rust-commands reminders` (session mode only) makes Rust own prepare, validation and the
conflict decision for the named sources. For their operations Swift never sees `Prepare`, `Dispatch` or a
ticket; it answers closed primitives and reports what the framework did.

| Frame | Direction | Fields |
| --- | --- | --- |
| `Primitive` | Rust to Swift | `id`, `op`, `args` |
| `PrimitiveResult` | Swift to Rust | `id`, `result` |
| `PrimitiveError` | Swift to Rust | `id`, `code` (`permission_missing`, `unsupported`, `invalid`, `conflict`, anything else is `unavailable`) |

One request is answered once; an answer after Rust gave up (10 seconds for reads, 30 for a write) is ignored,
an answer to an id Rust never sent closes the session. A primitive returns framework facts, never a
decision: Swift does not compare against `expected_state`, does not check a native state and does not judge
its own write.

Reminders primitives (`reminders.*` operations):

| Op | Args | Result |
| --- | --- | --- |
| `reminders.read_list` | `native_key` | `{exists, writable, data}` (`data` is the list's native fields) |
| `reminders.read_reminder` | `native_key` | `{exists, writable, list_key, data}` (`writable` is the owning list's) |
| `reminders.can_create_list` | none | `{allowed}` (the default account accepts a new list) |
| `reminders.reconcile` | none, or `absent_key` and `list` | `{}` after Swift emitted a fresh `Scan` of the source (used to evidence a conflict); with `absent_key` it emits the revalidated absence of that key instead (used to resolve a delete) |
| `reminders.write` | `op`, `native_key`, `list_key`, `args`, `command_id` (diagnostic only) | `{native_key, saved, observed, exists_after}`: `saved` is the record as written, `observed` a fresh read of it, `null` when absent. Swift emits its normal `Scan` of the change before answering |

Rust decides: unsupported operation or read-only list (`unsupported`); a missing reminder, a changed list or
reminder, an unexpected owning list (`conflict`, with `{native_key, capture_checkpoint}` evidence when the
fresh scan shows the key); drift between `Prepare` and `Dispatch` (rejected before any write, the dispatch
slot already written to the guard); and whether the read-back proves the write (`locally_observed`) or not
(`indeterminate`, never retried). Projection-only fields (store enrichment) never take part in a comparison.
`Resolve` after a restart is Rust too: for an open outcome of a Rust-owned operation Rust reads the list or reminder
through `reminders.read_list` / `reminders.read_reminder`, decides whether the framework shows the command's effect
(`locally_observed` with `resolved: true`) or not (`indeterminate`, evidence unchanged), and asks for
`reminders.reconcile` before answering. In session mode the Helper starts `ab-session` with
`--rust-commands reminders` (`AB_SESSION_RUST_COMMANDS` overrides the list), so Swift's native-session `Prepare`,
`Dispatch` and `Resolve` handlers refuse `reminders.*`.

## Ownership and migration

See rust/interfaces/ab-session.md. Exactly one external ipc/1 session and one authoritative
producer per source. Proxy mode relays authentication and every byte without decoding;
Swift keeps IDs, credits, Committed, checkpoints and durable command authority. Shadow
Rust readers receive no IPC writer and cannot spend credit or advance authoritative state.
They compare complete key/data snapshots with scope/generation. Partial snapshots are
inconclusive. A source switches only after draining all captures and disconnecting, then
reopens from its committed checkpoint. Routed Rust production is explicitly rejected by
this proxy until a separate routing implementation lands. No two-peer extension.
