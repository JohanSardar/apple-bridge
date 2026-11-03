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
| contacts.history | opaque history token; changes or token-invalid result | ContactsSource/ContactsSource.swift, ContactsHistory/ContactsHistory.m |
| ui.target | exact chat/handle/message key; verified native target facts | MessagesSource/MessagesAX.swift, MessagesAccessibility.swift, MessagesNativeSession.swift |
| ui.actions | qualified action names and readiness, no execution | MessagesSource/MessagesCommands.swift |
| focus.status | current authorization/observed status | FocusSource/FocusSource.swift |
| location.status | non-prompting authorization | MapsSource/LocationStatus.swift |
| mac.state | bounded state projection | MacSource/MacSource.swift |
| permissions.status | non-prompting permission states | AppleSession/Permissions.swift, NewPermissions.swift |

`Facts` fields: `id`, `op`, `args` object, `snapshot` (null on first page), `cursor`
(null on first page), `limit` (1..1000, default 100). `FactPage` fields: `id`, `op`,
`snapshot` nonempty opaque token, `items` array, `cursor` (next token or null),
`complete` boolean, `scope` nonempty string or structured object (including collections and history_token) and `generation` nonempty string.
Initial source adapters accept args={} for their full bounded source scope; unsupported nonempty filters return unsupported, never false completeness. contacts.history accepts history_token.
All pages of a snapshot pin one materialized read, scope, generation and argument
set. Continuations bind operation, args and snapshot; another request cannot reuse
a cursor. No page is larger than the frame limit; reduce count or use file refs.

`complete:true` means the terminal page exhausts the full requested scope, with no
failed, missing, timed-out or truncated reads. Nonterminal pages have complete false.
Truncated terminal pages have cursor null and complete false. Expired snapshots return
expired, never a replacement token. Invalidated Contacts history requires a new full
snapshot. Token advancement happens only after gateway Committed for the corresponding
capture. Notifications invalidate snapshots; they do not imply deletions. An optional
`Changed` frame carries `source` and `generation` as an invalidation hint only.
Absence is evidence only after all pages complete with unchanged scope/generation,
as required by SourceSupport/CompleteScan.swift and SQLiteSnapshot.swift.

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
| Vision/PDF | vision.ocr, pdf.text, pdf.render, pdf.annotate | VisionSource/VisionQueries.swift; PDFSource/PDFQueries.swift, PDFSource.swift |
| Speech/audio | speech.synthesize, speech.say, capture.play_sound | SpeechSource/SpeechQueries.swift, SpeechPlayback.swift; CaptureSource/CapturePlayback.swift |
| Maps/Location | maps.search, maps.nearby, maps.directions, maps.eta, maps.open, location.current, location.geocode, location.reverse_geocode | MapsSource/MapsQueries.swift, MapsCommands.swift |
| Focus | focus.set | FocusSource/FocusSource.swift |
| Capture/print | capture.screenshot, capture.camera_photo, capture.record_audio, mac.screen_record, print.document | CaptureSource/CaptureSource.swift, CaptureVideo.swift |
| Mac controls | mac.set_volume, mac.set_mute, mac.set_appearance, mac.launch_app, mac.activate_app, mac.quit_app, mac.keep_awake, mac.clipboard_read, mac.clipboard_write, mac.notify, mac.open_settings, mac.set_wallpaper, mac.network_check, mac.bluetooth_devices, mac.time_machine_status, mac.recognize_song, mac.iwork_export | MacSource/MacSource.swift, MacExtrasQueries.swift, MacNetworkQueries.swift, MacClipboardQueries.swift, MacIWork.swift |

These are wire adapters around existing implementations, not new capabilities.
Unsupported adapter operations return unsupported. mac.lock_screen, mac.sleep_display,
mac.restart and mac.shutdown remain unsupported, matching the current prepare rejection.
No permission request operation exists: permissions.status is facts only and never prompts.

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

## Ownership and migration

See rust/interfaces/ab-session.md. Exactly one external ipc/1 session and one authoritative
producer per source. Proxy mode relays authentication and every byte without decoding;
Swift keeps IDs, credits, Committed, checkpoints and durable command authority. Shadow
Rust readers receive no IPC writer and cannot spend credit or advance authoritative state.
They compare complete key/data snapshots with scope/generation. Partial snapshots are
inconclusive. A source switches only after draining all captures and disconnecting, then
reopens from its committed checkpoint. Routed Rust production is explicitly rejected by
this proxy until a separate routing implementation lands. No two-peer extension.
