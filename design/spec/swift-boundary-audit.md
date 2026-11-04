# Swift boundary audit (2026-11-04, started at main b640f8a19)

The owner's rule: Swift makes Apple framework calls and holds the macOS permissions. Rust holds every
decision. This audit sorts `native/Sources` by that rule. A line count is a result of the sorting, not its
goal.

A bundled `ab-session` is a posix_spawn child of the Helper without disclaiming responsibility (spike S2), so
it reads the same protected files under the Helper's TCC identity. "Needs TCC" alone is therefore not a reason
to stay in Swift. Only "needs an Apple framework API, or a permission only a process with a bundle identity
can hold" is.

## Must stay in Swift, and why

| Area | What | Why it cannot move |
| --- | --- | --- |
| App | `AppleBridgeHelper/*` (setup window, SMAppService, App Intents host, media-facts entry) | AppKit, ServiceManagement, AppIntents, and the bundle that holds the TCC grants |
| Permissions | `AppleSession/Permissions`, `NewPermissions`, `AppleBridgeHelper/SetupPermissions` | TCC status and request APIs. `SetupPermissions` opens protected SQLite files only to ask TCC whether Full Disk Access is held |
| EventKit | enumerate, revalidate, write and change notifications in `CalendarSource`, `RemindersSource`, `AgendaGate` | `EKEventStore` is the only API that writes and the only one with live identifiers |
| Contacts | enumerate, revalidate, write, change history in `ContactsSource`, `ContactsHistory` | `CNContactStore`, including the Objective-C history request |
| AppleScript | `NotesScript`, `NotesRichEdit`, `NotesFolderUI`, `MailCommands`, `SafariTabs`, `MacExtrasQueries` | `NSAppleScript` runs inside the identity that holds the Automation grant |
| Accessibility | `MessagesAXPrimitives`, `MessagesAccessibility`, `MessagesNavigation`, `MessagesUISession`, `MessagesNativeSession`, `MessagesAppleScript` | AX APIs and the IMCore route |
| Media | `MediaDerivatives`, `MediaExportGuard` | ImageIO, AVFoundation, QuickLook |
| Files frameworks | `ubiquity`, coordinated read, `transform` in `FilesSource` | `URLResourceValues`, `NSFileCoordinator`, PDFKit, ImageIO |
| System | `MacSource.state`, `observe`, command primitives | CoreAudio, IOKit, CoreWLAN, NSWorkspace, NSPasteboard |
| Focus | `INFocusStatusCenter` status and permission | Intents |
| Capture, speech, vision, audio, maps, PDF | their providers | ScreenCaptureKit, AVFoundation, Speech, Vision, MapKit, CoreLocation, PDFKit |
| Link | `NativeProtocol`, `NativeSessionLauncher`, `SessionProxy` | The Helper has to spawn the child and answer native/1 |

## Logic that has to move or go

| Area | What | Decision | Where it goes |
| --- | --- | --- | --- |
| Session | `AppleSession` capture loops, `emit`, `sendScans`, `split`, `ScanCommitted` waiters, checkpoints, `Source.capture`, `ObservationSink`, `CaptureOwnership`, `CaptureRetry`, `SessionSources.rustStore` | Delete | ab-session already owns chunks, credit, checkpoints |
| Store readers | `RemindersStore` (all), `CalendarStore*`, `*DatabaseHints`, `CaptureScope`, `CalendarScanScope` | Delete | `ab_native::reminders_store`, `calendar_store` |
| Baselines | `MessagesBaseline`, `*-baseline.json`, `notes-identities.json`, Calendar event stamps, Reminders generation file | Delete | `capture::State`, `session/capture-state` |
| Readers | `CallsSource`, `VoicemailSource`, `VoiceMemosSource`, `UsageSource`, `SafariSource` scans, `MailSource.snapshot`, `MessagesReader`, `MessagesWatcher`, `NotesDatabase`, `NotesChangeStore`, `NotesStore`, `NotesDeletion`, `FilesSource.snapshot`, `DatabaseWatcher`, `SQLiteSnapshot`, `CompleteScan`, `FileScanGuard` | Delete | `ab_native` readers, `ab_sys::watch` |
| Decoders | `Decoders` (typedstream, notes protobuf, message rich) | Delete with their last caller | `ab_native::typedstream`, `notes`, `rich` |
| Shortcuts inventory | `shortcuts list` driving and its diff | Move | New reader `readers::shortcuts`; the CLI needs no framework |
| Focus scan | `Assertions.json` decision and 30 s scan | Move | New reader `readers::focus`; Swift answers a `focus.status` fact with the framework values only |
| Safari tabs | The 5 s loop and its diff; tabs sent as a `Scan` | Move | `SafariReader` pulls a `safari.tabs` fact; Swift only runs the script |
| Readiness of file sources | `open()` doing schema checks for Phone, Voicemail, Voice Memos, Usage, Mail, Shortcuts | Move | `CaptureReader::readiness` |
| Notes reads | `notes.read`, `notes.observe` primitives over SQLite | Move | Session reads through `ab_native::notes_store` |
| Hint scans | `sendHint`, `captureCommand`, `reconcile(into:)` after a write | Delete | The session's `bind` already nudges and waits for the reader |

## Sources that capture through Swift today

From the live home: `files.item`, `mac.*`, `focus.status`, `shortcuts.*`, `voicemail.*`, `voice_memos.*`. Files
and Mac already have Rust readers that were never switched on. Voicemail and Voice Memos have readers too; this
Mac has no Voice Memos store and an empty Voicemail store, so their proof can only be fixture-level here.

`capture`, `location`, `maps`, `pdf`, `speech`, `vision`, `audio` publish readiness and no records, except
`pdf.artifact`, which a command writes.

## Record ids

| Source | Alias | Generation in Swift | Risk when Rust takes over |
| --- | --- | --- | --- |
| files | none | a new UUID per Helper start | Ids hang on `(generation, key)`. Generation adoption keeps the last generation's ids. The live home holds 158 alive `files.item` for 6 real items, because each old Swift restart wrote a new id for each item without tombstoning the last |
| mac | `mac/<collection>/<key>` | resumed | none, the alias is stable |
| focus | `focus/status/current` | a new UUID per start | none, the alias is stable |
| shortcuts | `shortcuts/<collection>/<key>` | resumed | none, the alias is stable |
| voicemail, voice_memos | `<source>/<generation>/<key>` | the store's `Z_UUID` | none, same store, same generation |

## Reminders and Calendar store readers

Live runs `facts` without `store`, so Swift enriches EventKit rows from SQLite before they reach Rust. Rust's
store readers exist and have been compared clean for Reminders on a copy. The default turns them on, which lets
`RemindersStore` and `CalendarStore*` go. The proof on a copy has to show this keeps data, not only ids.

## Default instead of a switch

`production::load` builds the configuration from the user's home. `session/rust-capture.json` becomes an override:
per-source fields merge over the defaults, so the file on the live home keeps working and a new source is picked
up without editing it. `"exclusive": true` limits the session to the named sources, for fixtures and private runs.

## What the gate checks

`native/boundary-allowlist.txt` lists what may mention SQLite or gateway concepts. Entries go as their code goes.
What it cannot see is logic that uses neither word, so this table and the final report have to say what is left.

## Where it ended

Swift captures nothing. `AppleSession` has no capture loop, no scan frames, no checkpoints; `ObservationSink`,
`Source.capture`, the store readers, the decoders and the baselines are gone. ab-session reads every source, and
the Helper answers facts and primitives.

Facts Swift still answers, all framework values: `agenda.*`, `contacts.*`, `files.ubiquity`, `mac.state`,
`focus.status`, `safari.tabs`, `messages.media`, `pdf.artifacts`, `accessibility.trusted`.

### Logic that is still in Swift

Nothing here reads a store or decides what a record is. It is command logic, outside capture, and it is still logic:

| Where | What it decides | Why it is still there |
| --- | --- | --- |
| `MailCommands` | argument checks, AppleScript text, read-back rules for `mail.*` | Prepare/Dispatch path not yet primitives. Moving it is the Reminders pattern; the AppleScript run stays |
| `MacSource.dispatch`, `prepare` | argument bounds, read-back checks for `mac.*` | Same. The framework calls stay; the bounds and comparisons can move |
| `FilesCommands`, `FilesSource.search`, `read` | path confinement, `mdfind` predicate, command read-back | Same. `mdfind` needs no framework; coordination and tags do |
| `ShortcutsSource` | runs the `shortcuts` CLI, bounds, timeout | No framework needed. It can move with `shortcuts.run` |
| `FocusSource` | `focus.set` argument rules, reading `Assertions.json` for its read-back, polling | Same pattern. Rust already has the parse (`ab_native::focus`) |
| `PDFSource`, `CaptureSource`, `SpeechPlayback`, `MapsCommands` | argument checks before a framework call | Thin, and each check sits next to the call it guards |
| `CalendarSource.lookup`, `knownAbsent`, `revalidate` | how to find an event occurrence again | Depends on EventKit identifiers; the data is `calendar-lookups.json`. Candidate for facts plus a Rust decision |
| `ContactsSource.makePhoto` scheduling | when to derive a photo | Thin: it derives on first sight and tells Rust the source changed |
| `AppleSession` Prepare, Dispatch, Resolve | routing for the sources above | Goes when the last of them becomes primitives |
| Readiness of framework sources | coverage text and capability lists in Swift `open()` | The state is a framework fact; the text could move |

### What the allowlist still holds

`native/boundary-allowlist.txt` has two entries, each with its reason. `SetupPermissions` opens protected stores only
to ask TCC whether Full Disk Access is held, which needs the Helper's identity. `ClockTimers` drives the Clock app with
Accessibility and reads its timer store back; that read is logic and moves with the Clock driver.

### Record ids on a real home

`packaging/dev/copy-proof.py` runs a build on a private copy of the live home and compares by record id. The `files.item`
ghosts in the table above remain, because deleting them is a data decision nobody has made. Voicemail was never captured
before this work: the Swift and the Rust reader both failed on a real store, because `ZRECORDUUID` is a BLOB there.

### The lab

`Scan` remains on the link only so the lab's simulated Helper can feed the session. Nothing in production sends it.
