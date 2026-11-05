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
| AppleScript | `AppleScriptRunner`, `NotesScript`, `NotesFolderUI`, `SafariTabs` | `NSAppleScript` runs inside the identity that holds the Automation grant |
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

### Commands (2026-11-04, after Swift stopped capturing)

Swift no longer takes `Prepare`, `Dispatch` or `Resolve`, and has no command handling. ab-session has a driver per
source and calls a primitive for each framework call. In a production ab-session a command for a source with no
driver is refused; a helper-fed `Scan` and helper-routed commands exist only with the `lab` feature, for the lab's
simulated Helper.

| Source | Moved to Rust | Stays in Swift, and why |
| --- | --- | --- |
| Mail | argument and recipient rules, AppleScript text, read-back, error text | `NSAppleScript` run: the Automation grant belongs to the Helper |
| Files | path confinement, create, write, move, rename, `mdfind`, read rules, staged blobs | tags (`URLResourceValues`), trash and iCloud calls (`FileManager`), coordinated read (`NSFileCoordinator`), transforms (ImageIO, PDFKit) |
| Mac | bounds, read-backs, settings panes, iWork script and checks, `tmutil`, `system_profiler` | AppleScript run, `NSPasteboard`, `UNUserNotificationCenter`, `NSWorkspace`, the wallpaper call, the IOKit sleep assertion and its timer |
| Shortcuts | the `shortcuts` CLI run, bounds, evidence | nothing |
| Focus | `focus.set` rules, the AB Focus run, read-back through `Assertions.json` | `INFocusStatusCenter` status |
| PDF | document rules, annotation bounds, artifacts, blobs | PDFKit inspect and annotate |
| Capture | argument rules, artifact verification, `lp` | ScreenCaptureKit, AVFoundation, `NSPrinter` names, PDFKit |
| Speech, sounds | text, name and volume rules | `AVSpeechSynthesizer`, `NSSound` |
| Maps | coordinate and mode rules, read-back | `MKMapItem` |
| Calendar | occurrence lookup, `calendar-lookups.json`, positive absence, present, missing or outside the range | EventKit queries and writes; `candidates` reports what EventKit says |
| Contacts | when a photo is derived | the derivation (ImageIO, AVFoundation) |
| All framework sources | coverage text, capabilities, state and reason | one raw status per source: authorized or not, language count |

### What is still logic in Swift

Small, listed so nobody has to guess.

| Where | What it decides | Why it is still there |
| --- | --- | --- |
| Queries: `PDFQueries`, `MapsQueries`, `SpeechQueries`, `VisionQueries`, `AudioQueries`, `MacClipboardQueries`, `MacNetworkQueries` | argument bounds next to the framework call | They answer read-only `Query` frames. Moving only the bound would need a Rust table per query and changes nothing they can do |
| `CalendarTime.chunks`, `identity`, `floating`, `recurrenceEnd` | the windows of a full enumeration, the record key's format, a recurrence's end date | The key must match every existing record id. Enumeration and the recurrence end run against EventKit objects |
| `CalendarSource.patch`, `requestedGoal`, `RemindersSource.patch`, `ContactsSource` writes | how JSON fields map to framework properties | That mapping is the framework call |
| `ContactsSource.groupCapabilities` | reads the group proof file for this OS build | It writes the proof it reads. Its result is a fact in the raw status |
| `MessagesAXPrimitives`, `NotesFolderUI` | how to find a control in another app's accessibility tree | AX calls have to run in the Helper. The decision to act is in Rust |
| `AppleSession` watchdog | polls status, doubling up to 30 s, and sends it when it changes | Timing of a framework read |
| `PackagedApp`, `SetupWindow`, `SetupPermissions` | onboarding flow and which permission to ask for | AppKit, ServiceManagement, TCC requests |

### What the allowlist still holds

`native/boundary-allowlist.txt` has one entry. `SetupPermissions` opens protected stores only to ask TCC whether Full
Disk Access is held, which needs the Helper's identity. `ClockTimers` is gone: only retired Go runs called it, and no
operation reached it.

### Permissions

ab-session is a `posix_spawn` child of the Helper with responsibility not disclaimed, so what moved keeps the
Helper's TCC identity. File changes, `mdfind`, `shortcuts`, `tmutil` and `system_profiler` ran for real on a Helper
matching the reference requirement, with no prompt logged. Everything that needs a grant by identity (Automation,
pasteboard, Calendar, Focus, Screen Recording, Microphone, Camera, notifications) stays a Swift call.

### Record ids on a real home

`packaging/dev/copy-proof.py` runs a build on a private copy of the live home and compares by record id. The `files.item`
ghosts in the table above remain, because deleting them is a data decision nobody has made. Voicemail was never captured
before this work: the Swift and the Rust reader both failed on a real store, because `ZRECORDUUID` is a BLOB there.

### The lab

`Scan`, and commands routed to a helper, compile only with ab-session's `lab` feature, so the lab's simulated Helper can
still feed a session. A production build refuses them. `make rust-test` runs those tests with the feature on.
