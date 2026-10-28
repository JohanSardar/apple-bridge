**iMCP → Apple Bridge port map — researched 2026-10-27**

Read [iMCP commit `1f3798de359cc9e5a83b551c4390b017fb33979d`](https://github.com/loopwork/iMCP/tree/1f3798de359cc9e5a83b551c4390b017fb33979d), including its pinned dependencies, rather than inferring capabilities from the README. This is static source research; no Mac, permissions, account, accessory, or live database tests were performed. Runtime compatibility and the proposed effort estimates are **unverified**. Issue reports are evidence of reported behavior, not independent reproductions. All iMCP source links below are pinned to that commit; issue states were read on the research date.

**Decision.** Copy the small native adapters, database readers, parsers, and validation routines selectively. Do not replace Apple Bridge's five existing app Sources with iMCP's services. iMCP largely performs individual queries or immediate actions; it does not provide our durable replication, identity reconciliation, command preparation, or effect observation. Mail and the three recording/history readers offer the largest new data coverage. Home has a substantial reusable implementation, but needs a separate signed Catalyst companion. Maps, location, weather, and Music catalog requests should remain on-demand operations.

**Reading the tables.** `R` means a read; `W` means an effectful command, including launching a composer, dialing, recording, or sounding an alert. `?` marks optional inputs; unspecified types are strings, `[]` arrays, `bool` booleans, `num` numbers, and `int` integers. Outputs describe the actual handler result, before iMCP's shared MCP serialization. `Graph<T>` means Ontology's compact JSON-LD graph, not a Bridge record; `ItemList` means the indicated `itemListElement` array and pagination/metadata. Model names below refer to the source-linked encoders, not an invented stable wire schema. iMCP's JSON schemas are often advertised rather than fully enforced; copy explicit validation too. The service sections collectively enumerate all 83 declared tools at the pinned commit, including Home tools beyond the abbreviated task list.

**The receiving architecture.** [Source.swift](../native/Sources/SourceSupport/Source.swift), [operations.go](../internal/contracts/operations.go), and [SPEC §§5–9](../design/SPEC.md#5-module-map-and-interfaces) govern the port. Each new persistent Source needs permission/capability readiness in `open`; initial, resumed, live and complete reconciliation in `capture`; `{collection,native_key}` observations and private blob handles; database/store generations; checkpoints acknowledged only after Go commits; and complete-scan evidence before absence. A query page, empty result, missing cloud file, denied permission, or timeout is never a complete deletion scan.

For writes, `prepare` captures target identities, native pre-state, expected version, payload fingerprint, and candidate set. `dispatch` acts once under the existing durable dispatch guard, then rereads Apple-owned state. `resolve` only rereads and never repeats the effect. Use `UIActor` for AppleScript/Accessibility and `AgendaGate` for EventKit. Existing implementations are not uniformly finished: for example [NotesSource.resolve](../native/Sources/NotesSource/NotesSource.swift) currently returns `.null`; do not describe the protocol's intended guarantees as proven for every existing Source.

The current operation contract has only locally served `read` and target-routed `command`. Live external queries need an explicit additive target-query IPC/descriptor contract, cancellation, deadline, freshness, and source-scoped authorization; they cannot honestly be advertised as replicated reads available offline on every node. Do not disguise harmless queries as mutation commands merely to reuse dispatch. Conversely, arbitrary Shortcuts and UI launches must not be labeled reads. This contract extension is a shared prerequisite for the on-demand services below; no code change is made by this research.

## Calendar

[App/Services/Calendar.swift][Calendar]; shared conversions in [EventKit+Extensions.swift][EventKitExt] and [Foundation+Extensions.swift][FoundationExt].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `calendars_list` | R | none | Array `{title,source,color,isEditable,isSubscribed}`; notably no calendar ID |
| `events_fetch` | R | `start?`, `end?`, `calendars?:string[]` (names), `query?`, `includeAllDay?:bool=true`, `status?:none/tentative/confirmed/canceled`, `availability?`, `hasAlarms?:bool`, `isRecurring?:bool` | `Graph<Event>` with event identifiers, dates, place, notes, availability and other EventKit→Ontology fields |
| `events_create` | W | `title,start,end`; `calendar?,location?,notes?,url?,isAllDay?:bool=false,availability?,alarms?:Alarm[],hasAlarms?:bool,isRecurring?:bool` | Created Ontology `Event`, identifier assigned |
| `events_update` | W | `identifier`; `title?,start?,end?,occurrence?,calendar?,location?,notes?,url?,isAllDay?:bool,availability?,alarms?:Alarm[],span?:thisEvent/futureEvents` | Updated `Event` with identifier |
| `events_delete` | W | `identifier`, `start?` (occurrence selector), `span?:thisEvent/futureEvents` | `{deleted:true,identifier}` |

`Alarm` is relative (`minutes:int`, optional `type=relative`), absolute (`type=absolute,datetime`), or proximity (`type=proximity,locationTitle,latitude:num,longitude:num,proximity?:enter/leave,radius?:num`); each accepts `sound?` and `emailAddress?`. Update permits null clearing for title/location/notes/URL/alarms. Availability values are the EventKit extension's named enum. Despite the README's recurrence wording and create's `isRecurring`, there is **no recurrence-rule input** in this Calendar tool schema; inspect the handler, not the marketing description.

**Mechanism and permissions.** Public [EventKit / EKEventStore](https://developer.apple.com/documentation/eventkit/ekeventstore): `requestFullAccessToEvents`, `calendars(for:.event)`, `predicateForEvents`, `events(matching:)`, `event(withIdentifier:)`, `save(_:span:commit:)`, `remove(_:span:commit:)`; `EKAlarm` and `EKStructuredLocation` use CoreLocation coordinates. No direct calendar SQLite access. Full Calendar TCC and `NSCalendarsFullAccessUsageDescription`; iMCP enables the sandbox calendars resource. Our existing EventKit authorization still applies despite FDA. Dependency: Ontology; shared JSON schema/tool machinery as listed below.

**Tricks and pitfalls.** Lenient ISO dates use local time if no timezone; date-only fetch ends include the whole local day. Invalid start dates now reject; date behavior changed in [#175](https://github.com/loopwork/iMCP/pull/175), [#223](https://github.com/loopwork/iMCP/pull/223), and [#249](https://github.com/loopwork/iMCP/pull/249), following [#118](https://github.com/loopwork/iMCP/issues/118). Update added in [#273](https://github.com/loopwork/iMCP/pull/273). Recurring occurrence lookup includes `/RID=` handling and a refetch; do not confuse occurrence time with a new start time. Names are weaker selectors than calendar IDs. No durable scan or write journal; API completion alone is weaker than our postcondition checks. Shared baseline macOS gates are summarized below.

**Port / duplicates — S for selective borrowing, L for replacement (unnecessary).** Keep [CalendarSource](../native/Sources/CalendarSource/CalendarSource.swift), its [postconditions](../native/Sources/CalendarSource/CalendarPostcondition.swift), [occurrence identity/stamps](../native/Sources/CalendarSource/CalendarEventStamp.swift), and [store enrichment](../native/Sources/CalendarSource/CalendarStoreEnrichment.swift). They already capture calendars/events, account for recurrence identity, and observe commands. Borrow date-only/DST cases, explicit alarm validation, and `/RID=` regression fixtures if our checks lack them. Keep calendar native keys and series/original occurrence identity behind Bridge aliases; never port title-only identifiers. Existing `calendar.calendar` and `calendar.event` remain captured collections; search/agenda runs over them. iMCP adds no reason for a second EventKit store or a separate native-query endpoint.

## Reminders

[App/Services/Reminders.swift][Reminders], [EventKit conversions][EventKitExt].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `reminders_lists` | R | none | List objects with `identifier,title,source,color,isEditable,isSubscribed` |
| `reminders_fetch` | R | `completed?:bool`, `start?,end?`, `lists?:string[]`, `query?` | `Graph<Reminder payload>`; completion, recurrence, alarm and list annotations |
| `reminders_create` | W | `title`; `notes?,url?,list?,start?,due?,recurrence?,priority?,alarms?` | Saved reminder payload with identifier |
| `reminders_update` | W | `identifier`; create fields optional, plus `completed?:bool` | Saved reminder payload |
| `reminders_delete` | W | `identifier` | `{deleted:true,identifier}` |
| `reminders_lists_create` | W | `title` | Created list `{identifier,title,source}` |
| `reminders_lists_rename` | W | `title`; `identifier?` or `name?` must select a list | Renamed list `{identifier,title,source}` |
| `reminders_lists_delete` | W | `identifier?` or `name?` must select a list | `{deleted:true,identifier}` |

Shared properties: priority `none/low/medium/high`; nullable notes/URL/start/due/recurrence/priority/alarms support clearing on update. Recurrence: `frequency=daily/weekly/monthly/yearly`, `interval?:int=1`, `daysOfTheWeek?:string[]` (including ordinal weekdays), `daysOfTheMonth?:int[]`, `monthsOfTheYear?:string[]`, `weeksOfTheYear?:int[]`, `daysOfTheYear?:int[]`, `setPositions?:int[]`, `end?:{date? or count?:int}`. Alarms accept relative minutes (including shorthand), absolute datetime, or proximity; proximity accepts a free-text `location` **or** `locationTitle,latitude,longitude`, with optional radius/enter-or-leave/sound/email. See the linked schema for exact nested alternatives. Payloads include identifier, text/description, dates, priority, completion state/date, recurrence, alarms, and list association; this is richer than simply returning an Ontology Thing.

**Mechanism and permissions.** `EKEventStore` reminder predicates / `fetchReminders`, `calendarItem(withIdentifier:)`, `save/removal` of `EKReminder` and `.reminder` `EKCalendar`; `requestFullAccessToReminders`. `MKLocalSearch` resolves named alarm locations. Full Reminders TCC, `NSRemindersFullAccessUsageDescription`, sandbox calendar resource; named-place resolution also uses network MapKit. Dependencies: Ontology, JSONSchema, OrderedCollections.

**Tricks and pitfalls.** List identifiers override names; ambiguous duplicate names now reject. Alarm/recurrence parsers validate before native mutation; omission differs from null. [#271](https://github.com/loopwork/iMCP/pull/271) and [#272](https://github.com/loopwork/iMCP/pull/272) add updates/list management and completion/recurrence detail. **Open [#315](https://github.com/loopwork/iMCP/issues/315)** reports `EKErrorDomain 29` on deletion after granting permission during the process lifetime, while create/update work; restart fixes the reported case. A pre-authorization singleton store being stale is the reporter's **unverified** explanation, not an established root cause. Recheck permission/store lifecycle on our `AgendaGate`; never auto-retry an uncertain delete.

**Port / duplicates — S–M.** Keep [RemindersSource](../native/Sources/RemindersSource/RemindersSource.swift) and [RemindersStore](../native/Sources/RemindersStore/RemindersStore.swift), including database enrichment/manual order, checkpoints, fingerprints, and readback. Capture remains `reminders.list` / `reminders.reminder`, with EventKit native IDs and current store generation. Borrow the strict recurrence/alarm/null parsers and duplicate-name rejection nearly as-is after replacing `Value` types. iMCP's list rename/delete are useful operation candidates beyond the SPEC's baseline list-create command; add typed contracts and observe list name, membership, and confirmed removal. Do not bypass existing richer reminders behavior with iMCP's narrower model. A successful `save` is not sufficient evidence.

## Contacts

[App/Services/Contacts.swift][Contacts], [Contacts+Extensions.swift][ContactsExt].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `contacts_me` | R | none | Ontology `Person?` for unified Me card |
| `contacts_search` | R | `name?,phone?,email?`; at least one valid selector | `Graph<Person>` |
| `contacts_list` | R | `limit?:int>=1` (all by default), `offset?:int>=0` | Stable-order paged `Graph<Person>` |
| `contacts_create` | W | `givenName`; other contact properties optional | Saved `Person` |
| `contacts_update` | W | `identifier`; contact properties optional | Saved `Person` |

Contact properties: `givenName,familyName,organizationName,jobTitle`; `phoneNumbers` and `emailAddresses` objects keyed by label; `postalAddresses` keyed by label with `street,city,state,postalCode,country`; `birthday:{day:int,month:int,year?:int}`. Reads also fetch relations, IM addresses, social profiles, URLs, contact type; notes and images are not included. No iMCP contact-delete tool at this commit.

**Mechanism and permissions.** Public [CNContactStore](https://developer.apple.com/documentation/contacts/cncontactstore), `unifiedMeContactWithKeys`, name/phone/email predicates, `unifiedContacts`, `CNContactFetchRequest`, and `CNSaveRequest`. Serial utility queue for store calls and complete fetch/edit/save; update requests mutable fetched contacts rather than constructing a new identity. Contacts TCC, `NSContactsUsageDescription`, sandbox `personal-information.addressbook`. Contact notes require separate access/entitlement considerations and are intentionally omitted. Dependencies: Ontology, JSONSchema, OrderedCollections.

**Tricks and pitfalls.** Multiple search selectors combine with AND; Contacts predicate support for every compound combination on our OS is **unverified**. Person conversion omits organization-only cards with no given/family name; people marked as companies are retained ([#206](https://github.com/loopwork/iMCP/pull/206)). [#148](https://github.com/loopwork/iMCP/issues/148) reports Cocoa 134092; [#231](https://github.com/loopwork/iMCP/pull/231) addresses editable fetch and serialization, explicitly without locally reproducing the framework error. Offset paging across concurrent mutations is not a sync cursor. `.authorized` checks do not model every newer limited-authorization behavior; verify on target OS.

**Port / duplicates — S.** Our [ContactsSource](../native/Sources/ContactsSource/ContactsSource.swift) and [ContactsHistory](../native/Sources/ContactsHistory/ContactsHistory.m) already cover richer fields, groups, history tokens, image blobs, create/update/delete, and observed writes. Retain unified/native identity aliases and `contacts.contact` / `contacts.group` capture. Borrow editable-fetch regression cases and optionally add an explicit Me-card relation/read operation, persisting the native Me-card association rather than repeatedly asking Contacts. Do not replace labeled arrays with dictionaries: duplicate labels are valid and dictionary projection loses information. Do not import the company-card omission as capture behavior.

## Messages

[App/Services/Messages.swift][Messages]; [Madrid Database.swift][MadridDB] and [FetchRequest.swift][MadridFetch].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `messages_fetch` | R | `participants?:string[]`, `chatId?`, `start?,end?,query?`, `isRead?:bool`, `limit?:int`, `attachments?:bool` | JSON-LD ItemList of messages with IDs, text, dates, sender/recipient/read state, conversation identity/participants; optional attachment metadata |
| `messages_attachment_read` | R | `id` (attachment GUID), `maxBytes?:int` (default 25 MiB; 1–100 MiB) | Media object `@id,name,encodingFormat?,contentSize,encoding="base64",content` |

**Mechanism and permissions.** Madrid's `iMessage` / TypedStream modules read `~/Library/Messages/chat.db` through SQLite, decoding attributed bodies. FetchRequest supplies chat, participant, dates, text and read-state filtering; service batches conversation metadata and optional attachments. Attachment SQL: `message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id`, restricted to fetched message GUIDs. File read resolves `SELECT filename,transfer_name,mime_type FROM attachment WHERE guid=? LIMIT 1`. No Messages framework history API, send tool, private injection, or SIP change. Apple [Foundation file access](https://developer.apple.com/documentation/foundation/filemanager), AppKit/SQLite3 plus Madrid (MIT); the SQL schema is not documented by Apple. Our FDA covers protected file reads; the read tools need no Automation/Accessibility.

**Sandbox-specific trick.** iMCP asks for **two** folder grants: `~/Library/Messages` for SQLite/WAL and `~/Library/Messages/Attachments` for media. Parent scope alone does not reach the protected attachment subtree in the reported sandbox tests. [#198](https://github.com/loopwork/iMCP/pull/198) fixes missing WAL history; [#247](https://github.com/loopwork/iMCP/pull/247) temporarily removed attachment reads; [#274](https://github.com/loopwork/iMCP/pull/274), fixing [#130](https://github.com/loopwork/iMCP/issues/130), restores them with the direct grant. The old wildcard absolute-path exception is not a substitute. The implementation validates real file paths, checks containment using the opened file descriptor, caps bytes, and detects a download growing beyond the cap. SQLite attachment reads have a 1-second busy timeout and mirror Madrid's fallback connection mode. An immutable fallback can miss WAL changes: never certify a complete Bridge scan from it.

**Port / duplicates — S.** [MessagesSource](../native/Sources/MessagesSource/Messages.swift), [reader](../native/Sources/MessagesSource/MessagesReader.swift), [watcher](../native/Sources/MessagesSource/MessagesWatcher.swift), [commands](../native/Sources/MessagesSource/MessagesCommands.swift), and our Decoders already exceed this read-only surface: durable chats/messages/handles/reactions/attachments, rich parsing, media and SIP-enabled writes. Keep GUID native keys and generation resets. Borrow attachment containment/growth tests and useful Madrid decoder fixtures only after comparing fidelity. iMCP output base64 is inappropriate for our IPC and replicated blobs; retain digest/blob streaming. Its filtered query results must not replace full capture or prove missing messages. [#305](https://github.com/loopwork/iMCP/pull/305) proposes sending; it is not a tool in the pinned source.

## Mail

[App/Services/Mail.swift][Mail], [MailTests.swift][MailTests]; implementation in pinned [Envelope Database][EnvelopeDB], [FetchRequest][EnvelopeFetch], [Store][EnvelopeStore], [MessageFileReader][EnvelopeFiles].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `mail_mailboxes_list` | R | none | ItemList of mailbox IDs, names/URLs and account metadata |
| `mail_messages_search` | R | `sender?,subject?,mailbox?,start?,end?,isRead?:bool,limit?:int=30` (1–100), `cursor?` | ItemList of message metadata; `nextCursor`; `localIndex:{indexedMessageCount?:int,syncStatus:"unknown",description}` |
| `mail_messages_read` | R | `id` from search | `EmailMessage` with ID, `headers:[{name,value}],text,encodingFormat,attachment:[{name,encodingFormat,isStoredSeparately}]` |
| `mail_compose` | W | `to?,cc?,bcc?,subject?,body?` | JSON-LD `CommunicateAction` describing a compose-URL launch after the default mail handler accepts `mailto:`; not a persisted draft or sent-message receipt |

Metadata includes `@id,mailbox,subject,sender.email,dateReceived,isRead`; content chooses readable body and lists attachments, **not** arbitrary attachment bytes. Search does not offer body FTS or a generalized query field.

**Exact storage mechanism.** Envelope discovers the newest suitable `~/Library/Mail/V*/MailData/Envelope Index` (not hard-coded V10), opens SQLite read-only with live WAL visibility, validates schema, and uses bounded read transactions. The page query selects `m.ROWID,m.mailbox,b.url,s.subject,a.address,m.date_received,m.read,m.message_id` from `messages m JOIN mailboxes b ON b.ROWID=m.mailbox LEFT JOIN subjects s ON s.ROWID=m.subject LEFT JOIN addresses a ON a.ROWID=m.sender`. Sender/subject match case-insensitive substrings; dates use Unix seconds with exclusive end; `m.read` filters read state. Mailbox membership includes `m.mailbox=? OR EXISTS(SELECT 1 FROM labels l WHERE l.message_id=m.ROWID AND l.mailbox_id=?)` when labels exist. Pagination orders `(date_received DESC,ROWID DESC)` with a keyset cursor; scope counts omit search filters. Mailboxes are `SELECT ROWID,url FROM mailboxes ORDER BY url,ROWID`.

`mail_messages_read` revalidates an Envelope ID (store version/mailbox/row), maps supported mailbox URLs to local `.mbox` storage, locates row-number `.emlx` files, parses the byte-count prefix and MIME body, decodes transfer encodings/charsets, and reports separately stored parts. `.partial.emlx` is a supported fallback when the complete file is absent. Containment/symlink and size checks matter. It does not fetch remote mail, force Mail to download bodies, or prove server completeness. [AppKit NSWorkspace.open](https://developer.apple.com/documentation/appkit/nsworkspace/open(_:)) opens a correctly percent-encoded `mailto:`; it may open a non-Apple mail client. No MailKit, IMAP credentials, Mail AppleScript, or private API is used by these tools.

**Permissions/dependencies.** iMCP grants `~/Library/Mail` through a read-only security-scoped bookmark; compose remains enabled when the user declines the read grant. Our FDA/direct path replaces the folder picker. No Mail Automation prompt for this implementation. Envelope 0.5.0 (MIT), Foundation/AppKit/SQLite3; no need to copy iMCP's MCP transport or Ontology dependency for this reader.

**Pitfalls.** Envelope defaults to 100 ms per statement, 1 second per snapshot, and zero SQLite busy wait. [#312](https://github.com/loopwork/iMCP/issues/312) reports large-store searches failing because scope COUNT exceeded the time budget even when the page was quick. [#314](https://github.com/loopwork/iMCP/pull/314) / Envelope 0.5.0, already pinned here, makes that count best effort: `indexedMessageCount=null` on count timeout; page timeout still fails. [#299](https://github.com/loopwork/iMCP/pull/299) adds partial-file support. Unsupported mailbox mappings and incomplete/offloaded MIME remain coverage gaps, not missing messages. V10 plus ROWID is insufficient across index rebuilds.

**Port — M for reads, L for dependable compose evidence.** Reuse Envelope's database/MIME/file readers nearly as-is; replace root authorization and JSON output. Add `mail.account`, `mail.mailbox`, `mail.message`, and attachment/blob metadata capture. Prefer native account/mailbox identity plus a generation-scoped local row; reconcile RFC Message-ID and other native identifiers as aliases only when unambiguous (duplicates and moves exist). Detect inode/schema/store replacement, observe WAL/file changes, re-read flags and labels, reconcile deletions with complete bounded scans, and hydrate bodies separately. This is a new Source; none of the five duplicates covers Mail.

Prepare compose against the selected app/account/recipients/body fingerprint. To promise an Apple Mail draft, add a targeted AppleScript/Accessibility composition route and observe the draft in Mail's own index/UI, accounting for delayed save and concurrent similar drafts. That is **new, unverified** functionality, not code supplied by `mail_compose`. If preserving iMCP's default-client launch, describe the narrow effect as composer-open request and retain indeterminate draft state when no Apple-owned record is observable. Never claim sent, delivered, or saved from `NSWorkspace.open == true`.

## Notes

[App/Services/Notes.swift][Notes]; pinned [TopoText store/decoder sources][TopoText].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `notes_folders` | R | none | ItemList of folder identities/names/account information |
| `notes_fetch` | R | `query?,folder?,start?,end?,limit?:int,cursor?` | Paged ItemList with note IDs/titles, folder/account, preview, created/modified dates, pinned/locked flags and `nextCursor?` |
| `notes_read` | R | `id`, `format?:markdown/plain` | `NoteDigitalDocument` metadata plus `encodingFormat,text`; attachments appear as bracketed text descriptions |

**Mechanism and permissions.** TopoText opens `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite`, snapshots SQLite and reads Core Data note/folder/account rows (`ZICCLOUDSYNCINGOBJECT`) and body rows (`ZICNOTEDATA`). Note queries join `ZICCLOUDSYNCINGOBJECT n` to folder `f` on `f.Z_PK=n.ZFOLDER`, filter native entity/deletion state and `n.ZIDENTIFIER`; folder queries join parent (`ZPARENT`) and account (`ZOWNER`). Body selection is `SELECT ZDATA FROM ZICNOTEDATA WHERE ZNOTE=? ORDER BY Z_PK DESC LIMIT 1`, or the schema-dependent join `d.Z_PK=n.ZNOTEDATA WHERE n.Z_PK=?`. Gzip + protobuf decoding reconstructs attributed text, checklists, links and table mergeable data; file readers locate local attachments under the Notes container. These are reverse-engineered storage formats accessed through SQLite and [Foundation file APIs](https://developer.apple.com/documentation/foundation/filemanager), not a public Notes framework. No AppleScript or Notes writes in iMCP. Dependency: TopoText 0.2.0 (MIT), Foundation/SQLite/zlib supplied by the platform. iMCP asks for the entire group container so WAL and assets are reachable; our FDA/direct reads replace this. No Automation TCC for the read path.

**Tricks and pitfalls.** Folder selector accepts identity or disambiguated name; query matches titles/content including tags, tables and image-recognized text. Dates filter modification time; default fetch omits Recently Deleted unless explicitly selecting that folder. This is search rather than a durable change feed. Locked and cloud-only bodies must remain explicitly unavailable. Pagination/order does not guarantee capture consistency across independent calls. [#282](https://github.com/loopwork/iMCP/pull/282) adds this service; [#311](https://github.com/loopwork/iMCP/issues/311) is a proposal for AppleScript writes, not implemented capability here. No service-specific bounded timeout beyond the underlying reader was established from the wrapper: deadline behavior for a large library is **unverified**.

**Port / duplicates — S–M.** Keep [NotesSource](../native/Sources/NotesSource/NotesSource.swift), [NotesChangeStore](../native/Sources/NotesSource/NotesChangeStore.swift), [body projection](../native/Sources/NotesSource/NotesBodyProjection.swift), and [NotesProto](../native/Sources/Decoders/NotesProto.swift). Our implementation already captures rich notes/folders/attachments, database changes, delayed hydration and plain-text writes with native readback. Borrow TopoText's Markdown/table rendering and format fixtures where better; compare locked/shared/offloaded behavior before adopting any decoder. Preserve Notes UUID native keys, full body/blobs and coverage rather than replacing them with a lossy Markdown-only record. Fill the existing resolve gap separately; this reader does not solve it.

## Voice Memos

[App/Services/VoiceMemos.swift][VoiceMemos]; pinned TSRP [Database][TSRPDB], [Movie][TSRPMovie], [Transcript][TSRPTranscript].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `voice_memos_fetch` | R | `query?,folder?,start?,end?,recentlyDeleted?:bool=false,limit?:int,cursor?` | Paged ItemList of recording IDs/name/date/duration/folder, local audio/transcript availability, deletion date; `nextCursor?` |
| `voice_memo_transcript_fetch` | R | `id`, `timestamps?:bool=false` | Object `{@id,text,inLanguage?}`, plus `segment:[{text,start?,end?}]` when timestamps requested |
| `voice_memo_audio_fetch` | R | `id`, `maxBytes?:int` (default 25 MiB, maximum 100 MiB) | Typed audio data (`audio/mp4` for m4a, other supported playable MIME types) |

**Exact mechanism.** Read `~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db` and adjacent files through TSRP 0.1.1 (MIT). SQL projects `ZCLOUDRECORDING r`: `Z_PK,ZUNIQUEID,title,ZDATE,ZDURATION,ZPATH,ZEVICTIONDATE`, optionally joining `ZFOLDER f ON f.Z_PK=r.ZFOLDER`. Schema probing tolerates optional columns; title favors text `ZENCRYPTEDTITLE`, then `ZCUSTOMLABEL`; folder title is `ZENCRYPTEDNAME`. Those “encrypted” columns may be plain local text. `ZEVICTIONDATE IS NULL` selects library; `IS NOT NULL` selects Recently Deleted. Filters use title contains, folder ID and Core Data dates; keyset cursor orders `ZDATE DESC,Z_PK DESC`.

Transcripts are **embedded in audio**, not produced by an ASR request: QuickTime/MPEG-4 `moov` → track user-data `tsrp` atom, or metadata key `com.apple.VoiceMemos.tsrp`. JSON contains Codable `AttributedString` runs with `timeRange=[start,end]` and locale. TSRP parses movie headers, verifies limits and containment, and reads only local files. Duration/title/transcription may be absent. iMCP accepts numeric row IDs and resolves UUID-style IDs. Public [Foundation file APIs](https://developer.apple.com/documentation/foundation/filemanager) and [UniformTypeIdentifiers](https://developer.apple.com/documentation/uniformtypeidentifiers) supply the platform layer; no Apple recording-history API is claimed.

**Permissions/pitfalls.** Folder bookmark covers DB/WAL/audio; our FDA replaces it. Reading old recordings requires no Microphone TCC. Device/cloud sync and locally generated transcripts are prerequisites; an absent audio file means transcript availability is unknown. [#295](https://github.com/loopwork/iMCP/pull/295) introduces the service. Playback format support is not equivalent to arbitrary recording format support. No live-library runtime validation performed; schema/OS coverage **unverified** beyond the source's documented layout.

**Port — M.** Depend on or vendor TSRP's Store/database/movie/transcript code, replace payload encoders, and add `voice_memos.folder`, `voice_memos.recording`, transcript fields and audio blobs. Use `ZUNIQUEID` when present; otherwise generation-scoped `Z_PK`, never filename/title alone. Watch DB/WAL **and audio files** because transcripts can arrive after metadata. Keep Recently Deleted as explicit state, hydrate large audio via blob handles, and capture actual timed segments. No writes exist; `prepare/dispatch/resolve` reject unsupported commands. No existing Source duplicates this collection.

## Voicemail

[App/Services/Voicemail.swift][Voicemail], [VoicemailStore.swift][VoicemailStore].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `voicemails_fetch` | R | `sender?,start?,end?,type?:carrierVoicemail/faceTimeAudio/faceTimeVideo,mailbox?:inbox/blocked/deleted/all,limit?:int` | ItemList of `@id,sender,date,duration,durationSeconds,isRead,transcribed,hasAudio,type,mailbox` |
| `voicemail_transcript_fetch` | R | `id` (schema string; handler also accepts integer) | Object `{@id,text,confidence?,segment?:[{text,confidence?}]}` |
| `voicemail_audio_fetch` | R | `id` | Typed `audio/amr` or `audio/mp4` data; rejects FaceTime video and unsupported formats |

**Exact mechanism.** macOS **26+** gate in `VoicemailService.isSupported`. `facetimemessagestored` synchronizes into `~/Library/Group Containers/group.com.apple.FaceTime/com.apple.facetimemessagestored/Data Store/FaceTimeMessageStore-local.sqlitedb`. SQLite reads `ZSTOREDMESSAGE` columns `Z_PK,ZFROM,ZDATECREATED,ZDURATION,ZISREAD,ZFILETYPE,ZRECORDUUID,ZTRANSCRIPT IS NOT NULL`; probes optional `ZPROVIDER,ZMESSAGETYPE,ZMAILBOXTYPE`. Dates are seconds since 2001. Inbox/blocked/deleted use mailbox values 0/1/2; deleted-row filtering also considers `ZDATEDELETED`. Type uses `com.apple.coretelephony` or a provider suffix `.FaceTimeProvider`, with `ZMESSAGETYPE=2` for FaceTime video; extension alone is wrong. Sender matching includes digit normalization.

Audio path is `Assets/<first two UUID characters>/<UUID>.<ZFILETYPE>` beside the database. The reader derives duration from QuickTime headers if the database reports zero. `SELECT ZTRANSCRIPT FROM ZSTOREDMESSAGE WHERE Z_PK=? AND ZDATEDELETED IS NULL` returns an NSKeyedArchiver plist; the decoder follows archived object references via [PropertyListSerialization](https://developer.apple.com/documentation/foundation/propertylistserialization) rather than instantiating arbitrary archived classes. Segments may carry confidence. No telephony private framework is injected or called. Dependencies: none beyond iMCP shared tool types and system Foundation/AppKit/SQLite3; the parser is in iMCP itself.

**Permissions/pitfalls.** iMCP needs a `Data Store` folder bookmark, not just the DB, and renews/holds it per operation ([#297](https://github.com/loopwork/iMCP/pull/297)); our FDA/direct access replaces it. No Microphone/Automation prompt for reading. iPhone/carrier/FaceTime sync determines available content; no guaranteed carrier archive. [#270](https://github.com/loopwork/iMCP/pull/270) fixes mailbox defaults, provider-based type, durations and formatted phone matching. The SQLite fallback retries immutable on SQLite errors and can omit WAL state; disable it for complete capture or mark coverage degraded. Transcript lookup excludes deleted rows even when deleted metadata can be fetched. Audio reads are not capped by a `maxBytes` parameter; add Bridge resource bounds.

**Port — M.** Copy the store, schema adaptation, transcript and movie parsing nearly as-is, with safer bounded blob access and explicit WAL consistency. Capture `voicemail.message`, audio and transcript under `ZRECORDUUID`; fallback to generation+row ID only if necessary. Watch database/assets and reconcile mailbox/read/transcript changes, not just new IDs. Record unknown provider/type, blocked/deleted status and offloaded audio faithfully. No write tools exist; do not infer mark-read/delete capabilities from raw SQLite access. No duplicate Source.

## Phone / call history

[App/Services/Phone.swift][Phone].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `phone_calls_fetch` | R | `participant?,start?,end?,call_type?:incoming/outgoing/missed,limit?:int` | ItemList of call IDs, participant number/name, date, duration, call direction/type and service provider |
| `phone_call` | W | `phoneNumber` | Ontology `CommunicateAction` describing a requested call after opening system dial UI |

**Exact mechanism.** Read `~/Library/Application Support/CallHistoryDB/CallHistory.storedata`, SQLite `SELECT Z_PK,ZADDRESS,ZNAME,ZDATE,ZDURATION,ZORIGINATED,ZANSWERED,ZSERVICE_PROVIDER FROM ZCALLRECORD`, with bound participant/date filters and descending date/ID ordering. Core Data epoch 2001. Outgoing=`ZORIGINATED=1`; incoming=`ZORIGINATED=0 AND ZANSWERED=1`; missed=`ZORIGINATED=0 AND ZANSWERED=0`. Dial path validates the phone string, constructs `tel:`, and calls [NSWorkspace.open](https://developer.apple.com/documentation/appkit/nsworkspace/open(_:)); system UI/Continuity asks the user to confirm. It does not use CallKit to retrieve historical calls or establish connection/delivery. Dependencies: Ontology plus Foundation/AppKit/SQLite3.

**Permissions/pitfalls.** iMCP grants CallHistoryDB directory, retaining compatibility with older file-only bookmarks; live SQLite/WAL read first, immutable fallback second. [#220](https://github.com/loopwork/iMCP/pull/220) addresses stale history from a DB-only grant; [#268](https://github.com/loopwork/iMCP/pull/268) addresses scoped bookmarks. Our FDA/direct path avoids these sandbox grants. Calling needs a configured system telephone handler and associated phone/account; no microphone prompt is required merely to open a URL, although the handling app has its own permissions. Call history contains only locally available synced records. No iMCP per-call completion timeout/readback proves that a call connected.

**Port — M for history; M–L for observed calling.** Copy `CallHistoryDatabase`, typed record/parser/filter code and phone validation. Capture `phone.call` using generation-scoped `Z_PK`; investigate a native durable call UUID before promising cross-rebuild identity (**unverified** from iMCP's selected columns). Watch WAL plus reconcile duration/answered-state updates. For dial command, prepare normalized number and candidate call IDs/time; dispatch the URL once; observe a new matching outgoing native record and report only the state it establishes. A matching row is not proof of remote answer; human cancellation, unrelated simultaneous calls and delayed sync can leave the command indeterminate. `resolve` may correlate later but never redial. No duplicate Source.

## Location

[App/Services/Location.swift][Location].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `location_current` | R | none | Ontology `GeoCoordinates` from CLLocation |
| `location_geocode` | R | `address` | JSON-LD `Place` object with `geo`, optional name and postal address from the first placemark |

**Mechanism/permissions/dependencies.** [CoreLocation CLLocationManager](https://developer.apple.com/documentation/corelocation/cllocationmanager), delegate callbacks, `requestWhenInUseAuthorization`, `startUpdatingLocation` / `stopUpdatingLocation`; `CLGeocoder.geocodeAddressString` for postal address lookup. The actual tool surface has no reverse-geocode endpoint despite broader README wording. iMCP configures kilometer accuracy initially, hundred-meter accuracy for a requested new fix. Location TCC and location usage strings; sandbox `personal-information.location`. Our unsandboxed Helper still needs Location authorization and a suitable packaged identity/run loop. Geocoding supplied addresses need not collect device location. Dependency: Ontology.

**Pitfalls.** A new fix has a 10-second wait; cached `latestLocation` is returned without an age check despite the “recent” comment. Authorization waiting is a separate lifecycle. [#224](https://github.com/loopwork/iMCP/pull/224) directs businesses/landmarks to Maps: geocoding a place name can resolve the wrong street. Accuracy, stale cache, network failures and authorization changes must be exposed. Current CoreLocation geocoder deprecation/migration on the deployment SDK needs implementation-time verification; retaining this pinned call does not certify future SDK behavior.

**Port — S–M.** Copy authorization single-flight/delegate flow and conversion intent; add cancellation, sample timestamp, horizontal accuracy, maximum cached age and bounded request lifetime. Keep both tools **on demand**, with no synced location-history collection. New target-query IPC is required. No write evidence needed and no duplicate Source; do not turn a one-shot location request into continuous tracking.

## Maps

[App/Services/Maps.swift][Maps], [MapKit+Extensions.swift][MapKitExt].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `maps_search` | R | `query`; `region?:{latitude:num,longitude:num,radius?:num}` | Array of Ontology `Place` results |
| `maps_directions` | R | Origin address or `{latitude,longitude}` in `originCoordinates`; destination address or `destinationCoordinates`; `transportType?:automobile/walking/any` | Ontology `Trip` from route response, including route/distance/travel information |
| `maps_explore` | R | `category,latitude:num,longitude:num`; `radius?:num,limit?:int` | Array of nearby `Place` results |
| `maps_eta` | R | `originLatitude,originLongitude,destinationLatitude,destinationLongitude:num`; `transportType?:automobile/walking/transit` | `Trip` from ETA response with travel-time/distance information |
| `maps_generate` | R | `latitude,longitude,latitudeDelta,longitudeDelta:num`; `width?:int,height?:int,mapType?:standard/satellite/hybrid/mutedStandard,showPointsOfInterest?:bool or category[],showBuildings?:bool` | PNG image data |

**Mechanism/permissions/dependencies.** Public [MapKit](https://developer.apple.com/documentation/mapkit): `MKLocalSearch.Request` / `start`, `MKLocalPointsOfInterestRequest`, `MKPointOfInterestFilter`, `MKDirections.calculate` / `calculateETA`, `MKMapSnapshotter.start`. Address resolution uses local search helpers; no user Maps favorites/history DB is read. Supplied coordinates do not require Location TCC; network access is necessary. Sandbox network-client access is distinct from our unrestricted network environment. Apple's [capability matrix](https://developer.apple.com/help/account/reference/supported-capabilities-macos/) notes Developer Program membership is needed for routing directions; verify our signed App ID provisioning for distribution. Dependencies: Ontology and system Foundation/AppKit image conversion through extensions.

**Pitfalls.** [#222](https://github.com/loopwork/iMCP/pull/222) removes unsupported transit turn-by-turn directions, while `maps_eta` still exposes transit. Preserve this distinction. `maps_directions`' top-level schema lacks required endpoint fields, although its handler requires resolvable endpoints; enforce in Go. Geometry, radius, count and image dimensions need explicit caps; wrapper continuations generally lack their own timeout. Backend quotas/region support and route freshness remain runtime **unverified**. Generated maps require preserved provider attribution, not cropping the rendered credit away.

**Port — S–M.** Copy request construction, POI enum mapping and snapshot conversion; replace Ontology with typed route/place results and digest-backed image artifacts. Keep all five tools **on demand**, cancellable and deadline-bounded; never treat route results as captured account state. No write confirmation and no duplicate Source. Share address resolution with reminder proximity alarms only through a narrow interface.

## Weather

[App/Services/Weather.swift][Weather].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `weather_current` | R | `latitude,longitude:num` | Ontology `WeatherConditions` |
| `weather_daily` | R | Coordinates; `days?:int=7` (clamped 1–10) | `Graph<WeatherForecast>` daily values |
| `weather_hourly` | R | Coordinates; `hours?:int=24` (clamped 1–240; handler also accepts numeric doubles) | `Graph<WeatherForecast>` hourly values |
| `weather_minute` | R | Coordinates; `minutes?:int=60` (clamped 1–120) | `Graph<WeatherForecast>` minute values; unavailable forecast errors |

**Mechanism/permissions/dependencies.** [WeatherKit](https://developer.apple.com/documentation/weatherkit) `WeatherService.shared.weather(for:including:)` with `.current`, `.daily`, `.hourly`, `.minute`, using supplied `CLLocation`. No weather database, Location prompt, or user Weather app history. `com.apple.developer.weatherkit`, a provisioned App ID with the WeatherKit capability/service, Developer Program membership and network access; FDA does not grant it. macOS framework minimum 13, below both projects' baselines. Ontology converts conditions/forecasts (temperature, precipitation, wind, humidity and applicable timestamps/units).

**Pitfalls.** [#191](https://github.com/loopwork/iMCP/issues/191) reports recurring genuine backend 401 authorization failures; [#171](https://github.com/loopwork/iMCP/issues/171) includes Weather authorization problems. A signed entitlement is not proof the backend will authorize every request. Some coordinate handlers accept only `.double`, rejecting mathematically integral JSON numbers; normalize numeric inputs. Minute forecasts are region/availability dependent and may contain fewer entries than requested. No explicit network timeout/retry policy is supplied by these wrappers. Apple's [WeatherAttribution](https://developer.apple.com/documentation/weatherkit/weatherattribution) is a publishing requirement separate from MIT (iMCP adds static attribution properties to forecast graphs); include the mark/legal link and preserve alert attribution if alerts are later added.

**Port — S implementation, M provisioning/verification.** Copy the four native queries; type outputs directly, include units/forecast issue and validity times, attribution, and unavailable-coverage states. Keep **on demand** with a bounded TTL cache, not replicated forecasts masquerading as user records. No mutations or duplicate Source. Verify entitlement/backend operation before advertising capability; throttle/retry transient queries without converting 401 into an empty forecast.

## Music

[App/Services/Music.swift][Music], [MusicTests.swift][MusicTests].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `music_now_playing` | R | none | `{state,title,artist,album,duration,position}` (seconds) |
| `music_control` | W | `action:play/pause/playpause/next/previous/stop/seek`; `position?:num` required for seek | `true` after AppleScript returns |
| `music_catalog_search` | R | `term`; `types?:string[]`, `limit?:int,offset?:int,includeTopResults?:bool` | Object of typed catalog arrays with IDs, names and relevant artist/album/artwork/URL/duration fields |

Types are `songs,albums,artists,playlists,musicVideos,stations,topResults`; the last also enables top-result grouping. These are Apple Music catalog results, **not** local library sync.

**Mechanism/permissions/dependencies.** `NSAppleScript.executeAndReturnError` controls application `Music` and reads `player state`, `current track`, `duration`, `player position`. Uses [Foundation NSAppleScript](https://developer.apple.com/documentation/foundation/nsapplescript) / Apple Events, not private MediaRemote. Catalog uses [MusicKit](https://developer.apple.com/documentation/musickit), `MusicAuthorization.request`, `MusicCatalogSearchRequest.response`, Song/Album/Artist/Playlist/MusicVideo/Station. Playback needs Automation consent for Music and `NSAppleEventsUsageDescription`; iMCP has the sandbox temporary Apple Events exception for `com.apple.Music`. Catalog needs Media & Apple Music consent and `NSAppleMusicUsageDescription`; verify MusicKit App ID/service configuration for our signing identity. The pinned entitlements do not justify inventing an additional arbitrary “MusicKit entitlement.” For our hardened runtime build, verify the [Apple Events entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.automation.apple-events) alongside TCC. No third-party data package is directly imported here.

**Pitfalls.** `-1743` becomes an actionable Automation error. Seek validates finite numbers, clamps to duration, and formats using POSIX decimal notation. AppleScript runs on `@MainActor` at this pin; [#307](https://github.com/loopwork/iMCP/pull/307) proposes moving it off the main thread, not evidence that this pin already does so. No playback readback follows `music_control`. Stopped playback returns empty track fields. Catalog availability/subscription/storefront failures need truthful errors.

**Port — S–M.** Copy typed catalog mapping, playback scripts and seek-validation tests; run all scripts through our serialized UI execution route with deadlines. Keep catalog and now-playing **on demand**; a future captured library would require new APIs and stable track IDs absent here. For commands prepare current player/track/position, dispatch once, reread Music state and compare desired state (seek tolerance, track identity for next/previous). Improve readback with persistent track ID: titles are not unique. Toggle/next are not safe to retry after a lost response. Report indeterminate if playback changes concurrently or no postcondition distinguishes the action. No duplicate Source.

## Shortcuts

[App/Services/Shortcuts.swift][Shortcuts].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `shortcuts_list` | R | none | Array of shortcut names |
| `shortcuts_run` | W | `name`, `input?:string` | `{success:true,shortcut,output?:string}` after CLI exit zero |

**Mechanism/permissions/dependencies.** Foundation `Process`, executable `/usr/bin/shortcuts`, argument vectors `list` or `run <name> --output-path <temporary file>` plus optional `--input-path <temporary text file>`. Temporary files removed in defer; nonzero status throws with stderr. No shell interpolation, AppleScript, direct Shortcuts DB, or external Swift package except JSONSchema. [Apple's CLI guide](https://support.apple.com/guide/shortcuts-mac/run-shortcuts-from-the-command-line-apd455c82f02/mac) confirms input/output flags and that interactive actions can pause the CLI. Shortcuts actions may need their own per-app data, Automation and execution permissions; our FDA does not bypass those. iMCP's sandbox/process behavior is not a general entitlement to arbitrary automation.

**Pitfalls.** Five-minute run timeout calls `terminate`; it does not undo partial effects or prove no effect. List lacks an equivalent timeout. `runProcess` assigns the termination handler after launch, and pipes are drained after termination: inspect early-exit and large-output/deadlock behavior when porting (**unverified** code-review risks). Returned output is UTF-8 text only; arbitrary binary outputs disappear. [#160](https://github.com/loopwork/iMCP/issues/160) reports unrestricted execution without per-invocation confirmation; Bridge's scoped commands must govern it.

**Port — S for list/runner, L for universal effect evidence (not generally achievable).** Copy safe argument construction and temporary-file lifecycle; replace process supervision with bounded concurrent stdout/stderr draining and durable execution tracking. List stays **on demand** because names are not stable native keys; UUID-based inventory needs separate verified discovery before becoming `shortcuts.shortcut` capture. Prepare shortcut identity/version and input fingerprint where obtainable; dispatch once. Exit zero proves CLI completion, not that a reminder/mail/home operation took effect. For named supported workflows, attach source-specific native observers and evidence predicates. Arbitrary shortcuts must expose process completion separately from business-effect verification and remain indeterminate for unobservable effects. No duplicate Source.

## Capture

[App/Services/Capture.swift][Capture], [AVFoundation extensions][AVExt], [ScreenCaptureKit extensions][SCExt], [ResumeGate.swift][ResumeGate].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `capture_take_picture` | W (sensor acquisition) | `format?:jpeg/png,quality?:num=.8,preset?,device?:built-in/continuity/external/desk-view,position?:unspecified/back/front,flash?:auto/on/off,autoExposure?:bool,autoFocus?:bool,autoWhiteBalance?:bool,delay?:num=1` | Typed captured image bytes |
| `capture_record_audio` | W (recording) | `format?:mp4/caf`, `duration?:num=10` (1–300 s), `quality?:low/medium/high` | Typed recorded audio bytes |
| `capture_take_screenshot` | R (screen acquisition; route as authorized live capture) | `contentType?:display/window/application,format?:png/jpeg,quality?:low/medium/high/max,displayId?:num,windowId?:num,bundleId?,includesCursor?:bool=true` | Typed screenshot image bytes |

Picture presets: `photo,low,medium,high,hd1280x720,hd1920x1080,hd4K3840x2160`; quality 0–1, delay 0–60 seconds. Capture does not write an item to Photos or Voice Memos. Classification here reflects effects and live privacy scope rather than trusting MCP hints.

**Mechanism/permissions/dependencies.** [AVFoundation](https://developer.apple.com/documentation/avfoundation/requesting-authorization-to-capture-and-save-media): camera discovery, `AVCaptureSession`, device input and `AVCapturePhotoOutput` delegate; `AVAudioRecorder` for temporary audio. [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit): `SCShareableContent`, display/window/application filter, `SCStreamConfiguration`, `SCScreenshotManager.captureImage`; AppKit/CoreGraphics image encoding. Camera/Microphone TCC with `NSCameraUsageDescription` / `NSMicrophoneUsageDescription`; sandbox/hardened-runtime camera and audio-input resource entitlements as applicable. Screen & System Audio Recording authorization for screen content; FDA/Accessibility do not replace it. No external Swift data package required.

**Pitfalls.** Audio format names are `mp4` and `caf`; recorder settings use AAC in either container. Camera and screenshot waits use 10-second timeout tasks and a one-resume gate. Camera schema allows a delay as long as 60 seconds, conflicting with a timeout started before the delay: **unverified runtime bug**, visible control-flow risk, no issue number established. Audio waits `duration+0.5`; requested device/mode may be unsupported. Modern screen capture is available under the project's baseline, but permission denial/empty sharing results are still possible. [#171](https://github.com/loopwork/iMCP/issues/171) includes screen-capture TCC failures. Validate output existence, MIME, duration/size and teardown; do not equate successful session start with captured bytes.

**Port — M.** Copy device/filter enums, encoder logic, and continuation gate; restructure ownership, cancellation and time budgets. Keep acquisitions **on demand**, return digest-backed immutable blobs with capture time/device/region metadata and retention policy. If artifacts are persisted, use command/request ID plus content digest for Bridge-owned artifact identity, not a fabricated Apple native key. Camera/audio completion evidence is actual captured media from the framework; no Apple app database record exists to observe. The contract must distinguish artifact production from modification of Apple-owned app state. Screenshot reads likewise need a live target query, not perpetual screen replication. No duplicate Source.

## Home

Shared [HomeTools.swift][HomeTools], native proxy [App/Services/Home.swift][Home], Catalyst [HomeKitBackend.swift][HomeBackend], [HomeKitStore.swift][HomeStore], [HomeModels.swift][HomeModels], and [HomeKit extensions][HomeExt]. **28 tools**, including automation management, not just rooms/scenes.

`home?` is required at runtime when more than one home exists. Object selectors are native UUID strings. `include_values?:bool=false` opts into live accessory reads. `Action={characteristic:UUID,value:bool/num/string}`; binary characteristic data uses base64 strings. Outputs below use the exact model names from HomeModels.

| Tool | R/W | Input | Output |
|---|---|---|---|
| `homes_list` | R | none | `{authorizationStatus:int,homes:HomeSummary[]}` |
| `home_export` | R | `home?,include_values?` | `{home,rooms,zones,accessories,scenes,automations}` |
| `rooms_list` | R | `home?` | `RoomSummary[]` |
| `zones_list` | R | `home?` | `ZoneSummary[]` |
| `accessories_list` | R | `home?,room?,category?,reachable?:bool,bridged?:bool,in_default_room?:bool,name_contains?` | `AccessorySummary[]` |
| `accessories_get` | R | `accessory,include_values?` | `AccessoryDetail` |
| `characteristics_read` | R | `characteristics:UUID[]` | Per-ID `BatchResult[]` with `ok,value?` or `error?` |
| `scenes_list` | R | `home?` | `SceneSummary[]` |
| `automations_list` | R | `home?` | `AutomationSummary[]` |
| `accessories_identify` | W | `accessory` | `AccessorySummary` after identify request |
| `accessories_rename` | W | `accessory,name` | `AccessorySummary` |
| `services_rename` | W | `service,name` | `ServiceSummary` |
| `accessories_assign_room` | W | `room,accessories:UUID[]` | Per-accessory `BatchResult[]` |
| `accessories_remove` | W | `accessory` | Removed-ID result |
| `rooms_create` | W | `name,home?` | `RoomSummary` |
| `rooms_rename` | W | `room,name` | `RoomSummary` |
| `rooms_remove` | W | `room` | Removed-ID result; accessories move to Default Room |
| `zones_create` | W | `name,home?,rooms?:UUID[]` | `ZoneSummary` |
| `zones_update` | W | `zone,name?,add_rooms?:UUID[],remove_rooms?:UUID[]` | `ZoneSummary` |
| `zones_remove` | W | `zone` | Removed-ID result |
| `characteristics_write` | W | `characteristic,value` | `{id,value}` echo after native write callback |
| `scenes_create` | W | `name,actions:Action[],home?` | `SceneSummary` |
| `scenes_update` | W | `scene,name?,add_actions?:Action[],remove_characteristics?:UUID[]` | `SceneSummary` |
| `scenes_execute` | W | `scene` | `SceneSummary` after execute callback |
| `scenes_remove` | W | `scene` | Removed-ID result |
| `automations_update` | W | `automation,name?,enabled?:bool` | `AutomationSummary` |
| `automations_remove` | W | `automation` | Removed-ID result |
| `automations_create` | W | `name,scenes:UUID[],trigger,home?` | `AutomationSummary` |

Trigger alternatives: `{fire_at:ISO timestamp,recurrence?:"daily" or weekday int[]}` (future whole-minute timestamp with timezone, weekdays 1 Sunday–7 Saturday), `{characteristic:UUID,value}`, or `{event:sunrise/sunset,offset_minutes?:int}`. New automations are enabled. `HomeSummary` has UUID/name/primary flag and room/zone/accessory/scene/automation counts. Room summary adds default-room flag and accessory count; zone adds room IDs. Accessory summary has home/room UUIDs, category, manufacturer/model/firmware, reachability/bridge metadata and service summaries. Detail adds characteristics with type, properties, metadata limits, optional value/error. Scenes have ID/name/type/actions. Automations have ID/name/kind/enabled/scenes/details and explicit `lastFireDate:null` plus limitations.

**Mechanism.** Public [HomeKit](https://developer.apple.com/documentation/homekit), [HMHomeManager](https://developer.apple.com/documentation/homekit/hmhomemanager) inventory/delegates; `HMHome`, `HMRoom`, `HMZone`, `HMAccessory`, `HMService`, `HMCharacteristic`; asynchronous name/room/membership/create/remove APIs; `readValue`, `writeValue`, `identify`; `HMActionSet` / `HMCharacteristicWriteAction` for scenes; timer/event triggers (`HMTimerTrigger`, `HMEventTrigger`, characteristic and significant-time events) for automations. No Home database, private injection or SIP changes. The native AppKit service proxies MCP over a paired local TLS connection to Catalyst. JSONSchema and shared MCP/network types are dependencies; no separate third-party HomeKit wrapper.

**Permissions and helper boundary.** Home TCC belongs to the companion, which requires `com.apple.developer.homekit`, `NSHomeKitUsageDescription`, provisioned signing and an iCloud account with access to the home; owner/admin rights may be needed for edits. Its sandbox entitlements include network client/server. Local Network usage/Bonjour declarations support discovery. FDA and Accessibility in our main Helper cannot grant HomeKit access or make HomeKit linkable as a supported native AppKit framework. See the packaging decision below.

**Tricks and pitfalls.** Store load waits up to 10 s; live characteristic reads up to 5 s, maximum four outstanding. Proxy launch/connect paths have 15 s discovery/connect, 5 s pairing and 20 s first-read limits. Native mutation callbacks are not uniformly bounded like reads. Read-only proxy calls retry once on lost connection; writes never retry automatically. [#288](https://github.com/loopwork/iMCP/issues/288) reported unauthenticated loopback access; [#294](https://github.com/loopwork/iMCP/pull/294) supplies code pairing, Keychain-held key, TLS PSK/ECDHE and local-only discovery in this pin. Preserve that security boundary if copying transport. [#290](https://github.com/loopwork/iMCP/issues/290) reports lost launch arguments/stale port, motivating discovery resilience; [#291](https://github.com/loopwork/iMCP/issues/291) / [#292](https://github.com/loopwork/iMCP/pull/292) address Mac registration during helper build. HomeKit omits some Home app/Shortcuts automation details, and last-fire history is unavailable; do not claim a full-fidelity Home backup.

**Port — L.** Copy Home models, `HomeArguments`, characteristic format/range validation, backend calls and read-throttling nearly as-is, replacing iMCP `Value`/tool coupling. Keep Catalyst separate, authenticated to our owner session with explicit capability/version negotiation. Main `HomeSource` should capture `home.home/room/zone/accessory/service/characteristic/scene/automation` using HomeKit UUIDs, delegate-driven invalidation and full reconciliation. Capture topology/metadata; keep volatile sensor reads on demand unless we explicitly add timestamped bounded subscriptions. Never turn unreachable accessories into tombstones.

Prepare fingerprints of topology/actions/characteristic current values and metadata; dispatch once; refetch names/memberships/scenes/triggers and read characteristic values to prove postconditions. Scene execution needs per-action readback and partial evidence; callback success does not prove every device changed. Removing accessories requires verified disappearance from a complete home inventory. Identify flashes/beeps generally lack a readable postcondition: expose acknowledged request separately, with physical effect **unverified**. HomeKit calls can partially mutate multi-object updates before an error; preserve per-item evidence. No existing Source duplicates Home.

## Utilities

[App/Services/Utilities.swift][Utilities], [AppKit+Extensions.swift][AppKitExt].

| Tool | R/W | Input | Output |
|---|---|---|---|
| `utilities_beep` | W | `sound` (enumerated system sound) | Boolean result of sound playback request |

**Mechanism/permissions/dependencies.** [AppKit NSSound](https://developer.apple.com/documentation/appkit/nssound), named sounds and `play`; JSONSchema advertises the sound enum. No database, TCC data grant, Automation, Accessibility, or network needed. Names: `Basso,Blow,Bottle,Frog,Funk,Glass,Hero,Morse,Ping,Pop,Purr,Sosumi,Submarine,Tink` (handler default Sosumi despite required schema input). Its `readOnlyHint:true` is misleading for our command classification. [#283](https://github.com/loopwork/iMCP/pull/283) / [#296](https://github.com/loopwork/iMCP/pull/296) fix tool registration so Utilities survives service-menu updates; it is always available in iMCP.

**Port — S.** Copy enum/play helper and route through an explicit effectful utility operation. No capture collection or durable native key. Prepare requested sound; completion can establish only playback API acceptance/completion, not that a human heard it (mute/output routing). The strict Apple-state-evidence contract needs an explicit transient-effect result, or must leave audible effect unverified; inventing a synced “beep record” would not prove sound. No duplicate Source.

## Shared permissions, identity and runtime requirements

iMCP's [Xcode project][Project], [App entitlements][Entitlements], [debug entitlements][DebugEntitlements], [Info.plist][Info], and per-service grant helpers are authoritative together. Xcode `ENABLE_RESOURCE_ACCESS_*` settings synthesize entitlements: reading only the checked-in plist misses sandbox calendar, contacts, location, camera, audio and network permissions. Release has App Sandbox and hardened runtime, user-selected read/write access, and WeatherKit. Explicit exceptions include Apple Events to Music/Terminal, legacy wildcard Messages paths, and Sparkle Mach services; these are not all required by every service. Debug signing/entitlements differ; successful developer direct-file access is not evidence the sandboxed release can read the same path.

Bookmarks: `NSOpenPanel` grants a selected file/folder; `bookmarkData(options:[.withSecurityScope,.securityScopeAllowOnlyReadAccess])` persists an opaque grant in UserDefaults; resolve with `.withSecurityScope`, refresh stale bookmarks, `startAccessingSecurityScopedResource` for the whole operation and balance `stopAccessing...` in defer. The store scope must remain held across async reads; a DB-only grant misses WAL and external assets. Replacing a stale/unresolvable grant requires user selection. [Apple documents the scoped-resource lifecycle](https://developer.apple.com/documentation/foundation/nsurl/startaccessingsecurityscopedresource()). Some services have legacy/debug direct-read paths. iMCP derives the actual user's home via password-database helpers rather than assuming a sandbox home ([Foundation extension][FoundationExt], [#301](https://github.com/loopwork/iMCP/pull/301)).

Our unsandboxed Helper can remove powerbox/bookmark/activation UI from internal protected-store readers and open actual owner paths with FDA. Keep canonical containment checks, bounded reads, permissions reporting and read-only SQL. **FDA does not replace** Calendar, Reminders, Contacts, Location, Camera, Microphone, Screen Recording, Home, Music, or Automation consent, and does not authorize writes to private databases. Accessibility is needed only for routes that actually use UI automation, not all iMCP services. Keep SIP enabled.

Version baseline: iMCP README says macOS 15.3+, Xcode app deployment settings say 15.1; Helper Catalyst target uses iOS 18.0. This is a documentation/build-floor difference, not proof of runtime support on every version. Voicemail explicitly gates macOS 26. Our [Package.swift](../native/Package.swift) targets macOS 26 and Swift tools 6.3, so most lower API floors are already met; signed entitlement behavior, linked SDK availability and live private schemas still need device verification. Do not copy Swift concurrency/actor assumptions blindly. iMCP's [ResumeGate][ResumeGate] is useful for one-shot callbacks, while unrelated transport continuation/DNS issues ([#167](https://github.com/loopwork/iMCP/issues/167), [#192](https://github.com/loopwork/iMCP/issues/192), [#193](https://github.com/loopwork/iMCP/issues/193)) are another reason to retain our existing gateway/IPC rather than transplanting the entire app.

## Dependencies and license obligations

The [resolved manifest][Resolved] pins the following versions. Licenses were read from checked-out dependency revisions, not assumed from iMCP's MIT license. Apple frameworks/system SQLite/zlib are SDK/system dependencies, not packages to vendor. Most data Source ports need only Envelope, TSRP, selected TopoText code, or no third-party dependency; the MCP transport dependency tree is optional because our Go gateway already supplies it.

| Package | Pin | License at pin | Port relevance |
|---|---|---|---|
| envelope | 0.5.0 / `edccf6c528e9` | [MIT](https://github.com/mattt/Envelope/blob/edccf6c528e94ffcadf678faff50f1e8b96c76e3/LICENSE.md) | Mail index, MIME and local files |
| eventsource | 1.1.1 / `07957602bb99` | [Apache-2.0](https://github.com/mattt/eventsource/blob/07957602bb99a5355c810187e66e6ce378a1057d/LICENSE) | MCP transport; omit with existing gateway |
| jsonschema | 1.3.1 / `4c6f2467d5bc` | [MIT](https://github.com/mattt/JSONSchema/blob/4c6f2467d5bc72b062c7a9b7e4cd77a09635ea8b/LICENSE.md) | Shared tool schemas; replace with Go contracts if convenient |
| madrid | 0.5.0 / `11ec80cb6770` | [MIT](https://github.com/mattt/Madrid/blob/11ec80cb67700e47948f90cef6de38bce4dbfb12/LICENSE.md) | Messages SQLite + TypedStream; existing decoder comparison only |
| ontology | 0.11.0 / `7e3937c46057` | [MIT](https://github.com/mattt/Ontology/blob/7e3937c46057ec9afcfb347da2241033d311bd34/LICENSE.md) | JSON-LD projections; optional, Bridge records replace it |
| sparkle | 2.9.6 / `ac2def288cbf` | [MIT-style main license; bundled component notices also apply](https://github.com/sparkle-project/Sparkle/blob/ac2def288cbff5cfc7df3ffef6abdf45b72bcb0a/LICENSE) | iMCP app updater; not needed for service ports |
| swift-async-algorithms | 1.0.3 / `4c3ea81f81f0` | [Apache-2.0 with Swift runtime exception](https://github.com/apple/swift-async-algorithms/blob/4c3ea81f81f0a25d0470188459c6d4bf20cf2f97/LICENSE.txt) | MCP asynchronous transport dependencies |
| swift-atomics | 1.3.0 / `b601256eab08` | [Apache-2.0 with Swift runtime exception](https://github.com/apple/swift-atomics/blob/b601256eab081c0f92f059e12818ac1d4f178ff7/LICENSE.txt) | Transitive concurrency/networking |
| swift-collections | 1.1.4 / `671108c96644` | [Apache-2.0 with Swift runtime exception](https://github.com/apple/swift-collections/blob/671108c96644956dddcd89dd59c203dcdb36cec7/LICENSE.txt) | OrderedCollections in Contacts/Reminders/schema builders; also transitive |
| swift-log | 1.6.2 / `96a2f8a0fa41` | [Apache-2.0](https://github.com/apple/swift-log/blob/96a2f8a0fa41e9e09af4585e2724c4e825410b91/LICENSE.txt) | Transport/dependency logging |
| swift-nio | 2.99.0 / `f71c8d2a5e74` | [Apache-2.0](https://github.com/apple/swift-nio/blob/f71c8d2a5e74a2c6d11a0fbe324774b5d6084237/LICENSE.txt) | Network/transport dependencies |
| swift-sdk | 0.12.0 / `6132fd4b5b42` | [Transition: Apache-2.0 plus unrelicensed MIT contributions; docs CC-BY-4.0](https://github.com/modelcontextprotocol/swift-sdk/blob/6132fd4b5b4217ce4717c4775e4607f5c3120129/LICENSE) | MCP Swift transport/tools; optional except if retaining Home proxy transport |
| swift-service-lifecycle | 2.7.0 / `7ee57f99fbe0` | [Apache-2.0](https://github.com/swift-server/swift-service-lifecycle/blob/7ee57f99fbe0073c3700997186721e74d925b59b/LICENSE.txt) | Server lifecycle dependencies |
| swift-system | 1.4.2 / `a34201439c74` | [Apache-2.0 with Swift runtime exception](https://github.com/apple/swift-system/blob/a34201439c74b53f0fd71ef11741af7e7caf01e1/LICENSE.txt) | System/networking dependencies |
| topotext | 0.2.0 / `69aa77928f0e` | [MIT](https://github.com/mattt/TopoText/blob/69aa77928f0eb85d98dff819f5459e893294bd71/LICENSE.md) | Notes reader and rich-text renderer |
| tsrp | 0.1.1 / `f8cf47d66830` | [MIT](https://github.com/mattt/TSRP/blob/f8cf47d66830e6a6124cf112f13dcb4e34d49f16/LICENSE.md) | Voice Memos store/movie/transcript reader |

**Attribution to record when copying.** [iMCP LICENSE.md][License] is MIT, copyright **2025 Mattt (https://mat.tt)**. Preserve that copyright and the full permission/warranty notice with copied substantial portions, including distributed binaries' third-party notices; modification/proprietary use is permitted under that notice. Do not substitute the current app's “dododo” About-box copyright for the actual upstream license. Each dependency has its own notice: Envelope, TopoText and TSRP are copyright 2026 Mattt; Madrid/Ontology and the other packages retain their respective pinned notices. Check any vendored subdirectories/resources separately; Sparkle's LICENSE contains multiple component notices. The Swift SDK license expressly describes a transition, so “everything is MIT” would be false. EventSource is Apache-2.0 at this pin too, despite several other mattt packages being MIT.

Proposed repo convention, **not files created by this task**: `third_party/imcp/LICENSE.md`, one license file per actually copied dependency, and `third_party/README.md` recording repository URL, full revision, copied upstream paths, corresponding Bridge paths, local modifications and date. Include those notices in packaged app acknowledgments/distribution. Put a short provenance comment at each copied file's top pointing to the manifest; retain existing copyright headers. For Apache packages preserve license/NOTICE where supplied and identify modifications; don't add a dependency solely for its old JSON output projection. This is a technical license inventory, not a finding that Apple's SDK/service terms or user-data rights transfer with MIT. Apple Weather attribution and MapKit branding obligations remain separate.

## HomeKit Helper: workable paths and constraints

Apple DTS confirms [HomeKit is supported on Mac only through Mac Catalyst](https://developer.apple.com/forums/thread/822517). Apple's [HomeKit setup documentation](https://developer.apple.com/documentation/homekit/enabling-homekit-in-your-app) requires capability/App ID configuration and consent; the [entitlement reference](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.homekit) identifies `com.apple.developer.homekit`. The [macOS capability matrix](https://developer.apple.com/help/account/reference/supported-capabilities-macos/) has HomeKit checked under ADP, blank under Developer ID and free Apple Developer. Its raw HTML was checked because text extraction loses checkmark icons. Thus the native unsandboxed Swift executable cannot simply gain supported HomeKit access by adding a plist key, and an ordinary Developer ID-signed companion is not a documented distribution path.

**Workable local development path.** Build a Catalyst companion as an Xcode app target, with a unique team App ID, HomeKit capability, development certificate and Mac Catalyst provisioning profile including the Mac. Keep its HomeKit/sandbox/network entitlements and `NSHomeKitUsageDescription`; include local-network/Bonjour declarations if retaining discovery. Sign into the relevant Apple account/iCloud home on the Mac and grant Home access to that companion. Adapt [Scripts/build-helper.sh][BuildHelper] and [Scripts/install-helper.sh][InstallHelper]; they use automatic provisioning/device registration and permit team/bundle/destination selection. Profile/certificate expiry means periodic rebuilding; only team-registered devices are covered. iMCP's own [README helper instructions][Readme] explicitly leave free Personal Team operation **unverified**; do not promise it. This does not require SIP off or private injection.

**Possible broader distribution path.** A separately sandboxed Mac App Store Catalyst companion with HomeKit capability is architecturally plausible; the non-sandboxed Bridge could remain separately distributed and pair locally. Approval of this precise companion/remote automation design, HomeKit data handling under current program terms, and the production IPC packaging are **unverified**. A signed iOS/iPadOS companion with HomeKit could also be a different target, but background execution/availability and remote transport would need new design; it is not a drop-in port. Neither path lets us promise a bundled zero-setup Developer ID helper today.

**Narrow fallbacks.** User-installed Shortcuts invoking known Home scenes may offer limited commands through Apple's own app. That does not provide complete inventory, stable UUIDs, automation management, or native write evidence. Accessibility control of Home is also **unverified**, incomplete and fragile as a full replacement. Do not use private HomeKit injection as a workaround. Make the Home capability unavailable until the companion is signed, paired, authorized and loaded; keep other Sources usable without it.

**Boundary to retain.** iMCP has already done substantial work here: 8-character one-use pairing code, random long-term Keychain key, TLS PSK plus ECDHE, five failed code attempts causing rotation, local-only Bonjour, versioned protocol, no automatic write replay. Copy or replace with an equivalent authenticated, per-owner boundary; loopback alone is insufficient. Our main Helper remains responsible for command tickets/fencing/dispatch guard, while the companion must also persist enough command IDs/evidence to resolve a lost response without replay. HomeKit postcondition reads can prove observed state, not prove every physical effect or capture all Home app-private automations.

## Summary: captured state versus on-demand work

Collections listed as “new” below are proposals, not present contracts. `S` = focused adapter or existing-source enhancement; `M` = new Source/query integration with identity, permission and evidence work; `L` = cross-process packaging or ambiguous/general-purpose write semantics. These are relative engineering estimates, **unverified**, not elapsed-time commitments. Account provisioning, hardware coverage and high-quality native fixtures may dominate code size.

| Service | Persistent captured data | On-demand operations | Effort / best borrowing |
|---|---|---|---|
| Calendar | Existing calendars/events, series/occurrences | Replica agenda/search | S: alarm/date validation; retain Source |
| Reminders | Existing lists/reminders, enrichment | Replica reads; commands for list changes | S–M: recurrence/null/alarm validation |
| Contacts | Existing contacts/groups, Me association if added | Replica lookup | S: mutable fetch and regression cases |
| Messages | Existing chats/handles/messages/reactions/attachments | Replica history and blobs | S: attachment safety/decoder comparisons |
| Notes | Existing folders/notes/body/attachment blobs | Replica search/rendered reads | S–M: TopoText renderer/fixtures |
| Mail | New accounts/mailboxes/messages/attachment metadata and local body blobs | Compose command; hydration | M reads, L observed compose: Envelope |
| Voice Memos | New folders/recordings/transcripts/audio blobs | Blob retrieval | M: TSRP |
| Voicemail | New message/read/mailbox/transcript/audio state | Blob retrieval | M: copy SQLite/transcript parser |
| Phone | New call history | Dial command, later native evidence | M history, M–L dial evidence |
| Location | None by default | Current fix, address geocode | S–M: CoreLocation adapter |
| Maps | None | Places, directions, ETA, PNG | S–M: MapKit adapter |
| Weather | None; bounded cache only | Conditions/forecasts | S code / M provisioning |
| Music | None for this tool surface | Catalog, player state, playback commands | S–M: scripts and MusicKit mapping |
| Shortcuts | None until stable UUID inventory is verified | List/run with workflow-specific evidence | S runner / L arbitrary-effect evidence |
| Capture | Optional Bridge-owned artifact metadata/blobs | Camera/audio/screenshot acquisition | M: AVFoundation/ScreenCaptureKit |
| Home | New UUID topology/scenes/automations/characteristic metadata | Live values and all native commands | L: backend/models plus Catalyst |
| Utilities | None | Sound command | S: tiny AppKit wrapper |

## Summary: permission matrix

All private-store readers are read-only; FDA is the proposed Bridge route, not an entitlement inserted into a binary. Entitlements mentioned below apply when sandbox/hardened runtime or service provisioning requires them; confirm the final signed app's generated entitlements, not just a source plist.

| Service | iMCP authorization / sandbox access | Unsandboxed Bridge requirement |
|---|---|---|
| Calendar / Reminders | Calendar resource; separate full-access TCC/purpose strings | Keep each TCC grant and AgendaGate lifecycle |
| Contacts | Address-book resource, Contacts TCC/purpose string; notes excluded | Contacts grant; don't assume FDA grants contact notes |
| Messages | Messages + separate Attachments scoped folders | FDA; bounded contained file reads |
| Mail | Mail scoped folder, compose independently available | FDA for read; app handler for compose |
| Notes | Notes group-container scoped folder | FDA for reads; Automation for our existing writes |
| Voice Memos | Recordings scoped folder | FDA; no microphone permission for historical files |
| Voicemail | FaceTime Data Store scoped folder; macOS 26 | FDA and locally synced data |
| Phone | CallHistoryDB scoped folder/file migration | FDA for history; configured telephone app for dial |
| Location | Location resource + TCC/purpose strings | Location TCC; no historical tracking implied |
| Maps | Outgoing network; routing membership configuration | Network and provisioned routing capability as needed |
| Weather | WeatherKit service entitlement/provisioning/network | Same WeatherKit authorization, separate from FDA |
| Music | Apple Events Music exception + Automation; Music authorization for catalog | Automation + Music consent/service setup |
| Shortcuts | Child process; each shortcut action has its own access | Command scope plus action-specific permissions |
| Capture | Camera/audio resource + TCC; Screen Recording permission | Same sensor/screen consent; hardened runtime exceptions as applicable |
| Home | Catalyst HomeKit entitlement/profile + Home consent; network/discovery | Separate signed/paired companion; FDA is irrelevant |
| Utilities | None | None |

## Suggested port order

| Order | Work | Why / acceptance boundary |
|---|---|---|
| 1 | Mail read Source using Envelope | Largest new everyday corpus; preserve partial/offloaded coverage and bound index scans. Prove local identity/rebuild and read/label/deletion changes before promising full sync. |
| 2 | Voice Memos with TSRP | High reuse, contained read-only scope, useful searchable transcripts. Prove late transcript/audio hydration and Recently Deleted transitions. |
| 3 | Phone history + Voicemail reads | Share private-store capture infrastructure; useful communication history. Prove WAL visibility, UUID/generation handling and asset states on macOS 26. |
| 4 | Small improvements to existing five Sources | Borrow date/alarm/recurrence/rendering/safety cases without replacing stronger capture. Add Me association/list-management operations only where absent. |
| 5 | Target-query contract, then Maps + Location | Unlocks all nonreplicated tools cleanly; bounded requests, timestamps, cancellation and blob outputs. Keep device location explicitly scoped. |
| 6 | Weather and Music queries/playback | Small adapters; gate on service signing/authorization and observed playback postconditions. Keep catalog out of replica state. |
| 7 | Capture + Utilities | Useful live tools; prove actual artifact production and honest transient-effect status. Utilities can be an incidental S-sized addition earlier. |
| 8 | Shortcuts list/runner; explicit supported workflows first | Runner is small, generic write evidence is not. Do not advertise arbitrary effects as verified. |
| 9 | Mail compose and phone dial evidence | Launch wrappers are easy but saved-draft/call correlation is not. Preserve ambiguity and never resend/redial during resolve. |
| 10 | Home companion and full Source | High capability value, highest packaging/account/evidence cost. Prototype signing/pairing first; then topology capture; then observed writes. |

**What remains unverified before implementation.** All live Mac behavior; private schema compatibility on the owner's installed build; cross-rebuild identity matching for Mail and call history; signing/account entitlement success for Weather/Music/Maps; generic Shortcut business-effect verification; native evidence for system compose/dial/transient sound/identify; and App Store approval/distribution of a Home companion. The research gives exact starting code and bounded implementation decisions; it does not establish universal Apple data completeness or successful remote delivery.


<!-- Pinned source references. -->
[Calendar]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Calendar.swift
[Reminders]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Reminders.swift
[Contacts]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Contacts.swift
[Messages]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Messages.swift
[Mail]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Mail.swift
[Notes]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Notes.swift
[VoiceMemos]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/VoiceMemos.swift
[Voicemail]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Voicemail.swift
[VoicemailStore]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/VoicemailStore.swift
[Phone]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Phone.swift
[Location]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Location.swift
[Maps]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Maps.swift
[Weather]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Weather.swift
[Music]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Music.swift
[Shortcuts]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Shortcuts.swift
[Capture]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Capture.swift
[Home]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Home.swift
[Utilities]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Utilities.swift
[EventKitExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/EventKit+Extensions.swift
[FoundationExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/Foundation+Extensions.swift
[ContactsExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/Contacts+Extensions.swift
[AppKitExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/AppKit+Extensions.swift
[MapKitExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/MapKit+Extensions.swift
[AVExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/AVFoundation+Extensions.swift
[SCExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Extensions/ScreenCaptureKit+Extensions.swift
[ResumeGate]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Shared/ResumeGate.swift
[HomeTools]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Shared/Home/HomeTools.swift
[HomeBackend]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Helper/Services/HomeKitBackend.swift
[HomeStore]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Helper/Services/HomeKitStore.swift
[HomeModels]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Helper/Models/HomeModels.swift
[HomeExt]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Helper/Extensions/HomeKit+Extensions.swift
[MailTests]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Tests/imcp-serverTests/MailTests.swift
[MusicTests]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Tests/imcp-serverTests/MusicTests.swift
[Project]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/iMCP.xcodeproj/project.pbxproj
[Entitlements]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/App.entitlements
[DebugEntitlements]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/App.Debug.entitlements
[Info]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Info.plist
[Resolved]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/iMCP.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
[License]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/LICENSE.md
[BuildHelper]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Scripts/build-helper.sh
[InstallHelper]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Scripts/install-helper.sh
[Readme]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/README.md
[MadridDB]: https://github.com/mattt/Madrid/blob/11ec80cb67700e47948f90cef6de38bce4dbfb12/Sources/iMessage/Database.swift
[MadridFetch]: https://github.com/mattt/Madrid/blob/11ec80cb67700e47948f90cef6de38bce4dbfb12/Sources/iMessage/FetchRequest.swift
[EnvelopeDB]: https://github.com/mattt/Envelope/blob/edccf6c528e94ffcadf678faff50f1e8b96c76e3/Sources/Envelope/Mail/Database.swift
[EnvelopeFetch]: https://github.com/mattt/Envelope/blob/edccf6c528e94ffcadf678faff50f1e8b96c76e3/Sources/Envelope/Mail/FetchRequest.swift
[EnvelopeStore]: https://github.com/mattt/Envelope/blob/edccf6c528e94ffcadf678faff50f1e8b96c76e3/Sources/Envelope/Mail/Store.swift
[EnvelopeFiles]: https://github.com/mattt/Envelope/blob/edccf6c528e94ffcadf678faff50f1e8b96c76e3/Sources/Envelope/Mail/MessageFileReader.swift
[TopoText]: https://github.com/mattt/TopoText/tree/69aa77928f0eb85d98dff819f5459e893294bd71/Sources/TopoText
[TSRPDB]: https://github.com/mattt/TSRP/blob/f8cf47d66830e6a6124cf112f13dcb4e34d49f16/Sources/TSRP/Database.swift
[TSRPMovie]: https://github.com/mattt/TSRP/blob/f8cf47d66830e6a6124cf112f13dcb4e34d49f16/Sources/TSRP/Movie.swift
[TSRPTranscript]: https://github.com/mattt/TSRP/blob/f8cf47d66830e6a6124cf112f13dcb4e34d49f16/Sources/TSRP/Transcript.swift
