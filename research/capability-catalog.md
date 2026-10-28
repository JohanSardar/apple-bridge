**Apple Bridge capability catalog — researched 2026-10-27**

The owner's Mac can expose substantially more Apple data without disabling SIP. This catalog contains **234 capability rows**. The strongest additions are Mail, Photos, files, Safari, call history, voicemail, Voice Memos, location, Maps, Weather, Music and the Shortcuts runner. This is a mechanism and source review, not a device compatibility certification: no Mac or iPhone actions were executed, no accounts were created, and no credentials were requested. Existing coverage comes from [SPEC §6.2 and §9](../design/SPEC.md) and [features.yaml](../features.yaml); existing capabilities below are explicitly marked **baseline** or described as extensions.

**iMCP comparison.** Reviewed [loopwork/iMCP at `1f3798de359cc9e5a83b551c4390b017fb33979d`](https://github.com/loopwork/iMCP/tree/1f3798de359cc9e5a83b551c4390b017fb33979d), including every service and the shared Home tool definitions: all **84 registered tools** are represented below. Tool names below are literal registrations, not names inferred from marketing. “No” means no dedicated tool at that revision; `shortcuts_run` can indirectly reach additional installed workflows, but that is not dedicated feature parity. Home tools exist in source, but require the separately built Catalyst helper; iMCP excludes it from Developer ID releases. Music playback uses Apple events, while catalog search uses MusicKit. Mail composition opens a window rather than sending. These distinctions matter when estimating parity. [iMCP README][imcp] [Music implementation][i-music] [Mail implementation][i-mail] [Home implementation][i-home]

**Four routes, with different guarantees.** **M** means the Mac only. **1** means the same account's enabled iCloud sync; it moves supported data, not arbitrary commands, and has no immediate-delivery guarantee. Mail may instead use the account's IMAP/Exchange sync, explicitly labeled **1-like**. **2** means Continuity/cellular relay; it requires Apple's device/account/network configuration. **3** means an iPhone Message automation invokes a router Shortcut and returns an acknowledgment. **4** means a future iPhone companion receiving requests and executing public APIs locally. **H** denotes a Home accessory/home hub, outside these four phone routes; **UI** means a visible application workflow. These routes are proposals unless an existing repo scenario proves them.

**The Shortcuts relay is promising, not verified infrastructure.** Apple documents sender/content Message triggers and automatic execution, but individual actions may still require permission, foreground UI or an unlocked phone. Receiving a self-addressed iMessage sent from the Mac, extracting its input, running each action while locked, and returning an unambiguous reply all remain **unverified** here. Provision the router on the intended device, use a device target, command ID, expiry and replay protection, separate request/response prefixes, and never turn arbitrary message text into executable code. Test self-message delivery before relying on it. Turning off radios may prevent the reply or future recovery. A timeout means unknown outcome, not safe-to-retry. These are design inferences, not Apple delivery promises. [Message trigger][message-trigger] [Automatic execution][auto] [Background push limitations][push]

**Version-sensitive corrections.** There is no native macOS Clock-alarm framework API, but Apple has documented Mac Clock actions in Shortcuts since macOS 13: do not label Mac alarms impossible. HealthKit can link on macOS yet has no usable Mac Health store. Apple's 26 release notes add Screen Time's “Get App & Website Data”; its output/privacy behavior needs a real probe before replacing Biome. The 27 notes, updated October 26, also document new automation and app actions. Older guidance says personal automations do not sync; the 27 notes say they now do. Treat this as version-dependent, require explicit target-device enrollment, and test it on the owner's installed build rather than applying either statement universally. [Shortcuts 16/13][sc16] [Health architecture][health-about] [Shortcuts 26][sc26] [Older automation scope][auto-scope] [Shortcuts 27][sc27]

**Reading the tables.** **R/W** describes the actual data/action, not a promise of complete app access. **High** means a documented public API/CLI/action exists; **medium** means a database or application scripting dependency; **low** means UI automation, an undocumented mechanism, or an unverified complete route. Route 3 is always **low end-to-end** until tested, even when its local action is documented. Conditional companion/Home capabilities are not ready in the current native Helper. **Impossible** is scoped to the stated request and supported architecture, not a claim that every adjacent workflow is impossible. Private APIs/protocols are **out of policy**, even when they do not require injection or SIP changes. **Unverified** marks gaps in documentation, current-build behavior, hardware coverage or proposed UI work. Every row has mechanism evidence; an adjacent API link on an unverified row supports only the documented boundary, not the proposed automation. Value **3** = frequent/high-impact, **2** = useful, **1** = niche; these are product judgments, not measured demand. Prior-art “—” means no additional implementation verified. FDA = Full Disk Access; AE = Automation for the named app; AX = Accessibility. Existing grants do not imply Photos, Location, Camera, Microphone, Screen Recording, Health, Home or entitlement authorization.

## Communication

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| C01 Mail accounts/mailboxes | “Which inboxes do I have?” | R account and folder metadata | Read `~/Library/Mail/V*/MailData/Envelope Index`; probe store version; [Envelope/iMCP][i-mail] | FDA | M; 1-like account sync | Medium | `mail_mailboxes_list` [source][i-mail] | [apple-mail-mcp][mail-art] | 3 |
| C02 Mail search | “Find Alex's invoice.” | R sender, subject, dates, local IDs | Same read-only index; iMCP searches metadata, not bodies or server-only mail. [source][i-mail] | FDA | M; 1-like | Medium | `mail_messages_search` [source][i-mail] | [apple-mail-mcp][mail-art] | 3 |
| C03 Mail bodies | “Summarize this email.” | R downloaded MIME/body, attachment metadata | Local `.emlx` and index; incomplete downloads remain unavailable. [source][i-mail] | FDA | M; 1-like | Medium | `mail_messages_read` [source][i-mail] | [apple-mail-mcp][mail-art] | 3 |
| C04 Mail attachments | “Save the invoice PDF.” | R attachment bytes; W exported file | Mail AppleScript attachment save, or local files; separate-file MIME storage needs resolution. [implementation][mail-art] | AE Mail; FDA for file reads | M; synced mail 1-like | Medium | Partial: metadata only in `mail_messages_read` [source][i-mail] | [apple-mail-mcp][mail-art] | 3 |
| C05 Compose window | “Draft an email to Sam.” | W compose UI, not delivery | `mailto:` through `NSWorkspace`; subject/body/recipients. [source][i-mail] | Usually none for opening URL; AE for richer draft | M; saved account draft may sync 1-like | High for window | `mail_compose` [source][i-mail] | [apple-mail-mcp][mail-art] | 3 |
| C06 Send/reply/forward | “Reply that Tuesday works.” | R original; W outgoing mail | Mail AppleScript dictionary; correlate Sent evidence, do not equate AppleEvent success with delivery. [implementation][mail-art] | AE Mail; file access for attachments | M → recipients; Sent sync 1-like | Medium | No | [apple-mail-mcp][mail-art] | 3 |
| C07 Call history | “Who called yesterday?” | R caller, time, duration, direction | `~/Library/Application Support/CallHistoryDB/CallHistory.storedata`, read-only with WAL. [source][i-phone] | FDA | 1, only history that reached Mac | Medium | `phone_calls_fetch` [source][i-phone] | iMCP is direct reference | 3 |
| C08 Place cellular call | “Call Mum on my iPhone line.” | W dial request, not guaranteed connected call | Open `tel:`; Phone/FaceTime and cellular relay perform call; UI confirmation possible. [source][i-phone] [Apple relay][calls] | Configured relay; microphone for conversation | 2 | Medium | `phone_call` [source][i-phone] | — | 3 |
| C09 Answer/end/mute call | “Answer the call”; “Mute me.” | R visible call state; W UI | Phone/FaceTime AX controls; exact controls unverified. CallKit manages the app's own VoIP service, not arbitrary Phone history/control. [CallKit][callkit] | AX; microphone where used | M / 2 | Low | No | [Hammerspoon AX][hs] (generic) | 2 |
| C10 Voicemail list | “Any new voicemails?” | R sender, date, status | macOS 26+ `~/Library/Group Containers/group.com.apple.FaceTime/com.apple.facetimemessagestored/Data Store/FaceTimeMessageStore-local.sqlitedb`. [source][i-vm] | FDA | 1, Phone-sync dependent | Medium | `voicemails_fetch` [source][i-vm] | — | 3 |
| C11 Voicemail transcript | “What did the doctor say?” | R existing transcript, not guaranteed transcription | Parse synced voicemail store; missing transcript is unavailable. [source][i-vm] | FDA | 1 | Medium | `voicemail_transcript_fetch` [source][i-vm] | — | 3 |
| C12 Voicemail audio | “Play that voicemail.” | R downloaded audio | Store's adjacent `Assets` tree; locate through metadata. [source][i-vm] | FDA | 1 | Medium | `voicemail_audio_fetch` [source][i-vm] | — | 3 |
| C13 Voicemail delete/read state | “Delete this voicemail.” | W native voicemail state | Phone AX workflow; no public voicemail CRUD API established; unverified. Never write SQLite. [read-only reference][i-vm] | AX | M; propagation to phone unverified | Low | No | — | 2 |
| C14 FaceTime call | “FaceTime Alex.” | W launch audio/video call | Public `facetime:` URL; confirmation/foreground may be required. [Apple URL scheme][facetime] | Camera/microphone in FaceTime; AX if submitting UI | M; phone participation via service, not remote phone control | Medium | No dedicated tool | — | 2 |
| C15 Call recording/transcription | “Record my call.” | R available recording; W visible recording request | Phone UI where supported; no public silent cellular-recording API. AX sequence and storage coverage unverified. [Apple recording][call-record] | AX; app/region/device requirements; OS participant notice | M / 2 where supported | Low | No | — | 2 |

## Productivity

Mail appears here for organization rather than repeating the communication rows.

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| P01 Mail organize | “Archive these”; “Mark this unread.” | W mailbox, read/flag/junk/delete state | Mail AppleScript; never mutate Envelope Index. [implementation][mail-art] | AE Mail | 1-like | Medium | No | [macos-apps-mcp][apps-art] | 3 |
| P02 Mail full-text index | “Find the email mentioning the serial number.” | R downloaded bodies; W bridge-owned search index | MIME extraction + own index; iMCP does not offer body search. [source][i-mail] | FDA | M; downloaded 1-like data | Medium | No | [apple-mail-mcp][mail-art] | 3 |
| P03 Notes rich read, baseline extension | “Read the table and checklist.” | R formatting, tables, attachment references | `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite`; protobuf/CRDT decode, no locked-content bypass. [source][i-notes] | FDA | 1 | Medium | `notes_folders`, `notes_fetch`, `notes_read` [source][i-notes] | [Cloud Notes Parser][notes-art] | 3 |
| P04 Notes rich create/append | “Append a formatted meeting summary.” | W supported rich text without replacing existing structure | Shortcuts rich-text create/append; existing Helper plain-text restriction stays until proven. [actions][sc16] | Notes/Shortcut grant | 1; 3 optional | Medium; 3 low | No | [macos-apps-mcp][apps-art] (plain scripting) | 3 |
| P05 Notes tags/pins/move | “Tag this as work”; “Pin this note.” | W tag/pin/folder | Shortcuts Notes actions; Mac availability of each action unverified. [action list][sc16] | Shortcut/Notes; AX fallback | 1 or 3 | Medium local candidate; 3 low | No | [Cloud Notes Parser][notes-art] (read reference) | 2 |
| P06 Notes locked content | “Read my locked note.” | R only after owner unlock | No public bypass; Notes UI unlock then selected content/export, unverified AX extraction. [encrypted storage][i-notes] | User unlock; AX | M; 1 data | Impossible unattended bypass; low assisted | No | [Cloud Notes Parser][notes-art] (known-password cases only) | 2 |
| P07 Files enumerate/search | “Find my tax PDF.” | R paths, metadata, contents | `FileManager`, `NSMetadataQuery`/Spotlight; authorized local roots. [FileManager][files] [query][metadata] | Files & Folders/FDA as applicable | M; 1 for iCloud Drive | High | No | [orchard-mcp][orchard] | 3 |
| P08 Files create/edit/move/trash | “Save this in iCloud Drive.” | R/W files/directories | Foundation file operations and `NSFileCoordinator`; use mounted iCloud Drive, no private CloudDocs API. [files][files] [coordination][coord] | File access; iCloud enabled | 1 for Drive; M otherwise | High | No | [orchard-mcp][orchard] | 3 |
| P09 Download offloaded files | “Fetch the original presentation.” | W download request; R materialized file | `FileManager.startDownloadingUbiquitousItem`; report actual availability, not request success. [API][download] | File access; network/iCloud account | 1 | High | No | [orchard-mcp][orchard] (file layer) | 3 |
| P10 File tags/metadata | “Tag these receipts as expenses.” | R/W Finder tags and supported resource values | URL resource values such as `tagNamesKey`. [API][tags] | File access | M; iCloud tag propagation unverified | High local | No | [Hammerspoon][hs] (file automation) | 2 |
| P11 Safari local tabs | “List my tabs”; “Open this page.” | R title/URL; W open/close/navigate | Safari AppleScript dictionary; extension APIs alternative. [ScriptingBridge][sb] [implementation][safari-art] | AE Safari | M; iCloud may advertise tabs via 1 | Medium | No | [icloud-mcp][safari-art] | 3 |
| P12 Safari webpage content | “Summarize the current page.” | R page DOM/selection | Safari `do JavaScript` with explicit developer setting, or Safari extension host permission. [page API][safari-page] [implementation][safari-art] | AE + Allow JavaScript from Apple Events, or extension website grant | M | Medium | No | [MCPAppleScript][osa-art] | 3 |
| P13 Safari history | “Which article did I read yesterday?” | R visits/title/URL | `~/Library/Safari/History.db`; profile-specific stores must be discovered. [parser][safari-db] | FDA / Safari data permission on current OS | M; 1 for synced history | Medium | No | [mac_apt][safari-db] | 3 |
| P14 Safari bookmarks/read list read | “Find my saved recipes.” | R folders, URLs, reading state | `~/Library/Safari/Bookmarks.plist`, including Reading List nodes. [parser][safari-db] | FDA / Safari data permission | 1 | Medium | No | [mac_apt][safari-db] | 3 |
| P15 Safari bookmark writes | “Bookmark this in Travel.” | W bookmark/folder | Safari UI; no supported native general bookmark CRUD API found. AX unverified; do not overwrite sync plist. [Apple export boundary][safari-export] | AX | 1 | Low | No | [MCPAppleScript][osa-art] (generic UI/OSA) | 2 |
| P16 Safari Reading List add | “Save this to read later.” | W list entry | Safari dictionary/Shortcut if present, else AX; native macOS `SSReadingList` is unavailable (Catalyst differs). [API platforms][reading] | AE/AX or Shortcut grant | 1 / 3 | Medium candidate; unverified exact Mac action | No | [icloud-mcp][safari-art] | 2 |
| P17 Safari remote tabs | “What is open on my phone?” | R synced tab metadata, not remote DOM | `~/Library/Safari/CloudTabs.db` where present; schema/location vary. [parser][safari-db] | FDA | 1 | Medium | No | [mac_apt][safari-db] | 3 |
| P18 Safari phone tab operations | “Open this on my phone”; “Close its shopping tabs.” | R/W phone tabs | Route 3 with Safari actions; per-action locked-state behavior unverified. [actions][sc16] | iPhone Shortcut/Safari approval | 3 | Low | No | — | 2 |
| P19 Freeform export/read | “Summarize this board.” | R exported PDF/image; not semantic object graph | UI export then PDFKit/Vision; no public board CRUD API established. [Apple export][freeform-export] | AX; file access | M; board data 1 | Low export; high PDF parsing | No | — | 2 |
| P20 Freeform add files | “Add these mockups to my board.” | W board file items | Shortcuts action documented in OS 26; board selection/input behavior unverified. [actions][sc26] | Shortcut/Freeform | 1; 3 if phone executes | Medium candidate | No | — | 2 |
| P21 Freeform board/object editing | “Create a board”; “Move that sticky note.” | W supported board actions; object-level UI | Shortcuts board create/open/rename/update documented; arbitrary object edits still need unverified AX. [actions][sc18] [guide][freeform] | Shortcut grant; AX for UI | 1 / 3 | High documented action; low object UI | No | — | 2 |
| P22 Journal search/export | “Find my journal entries from June.” | R user-exported entries | Journal Mac export UI then parse; Journaling Suggestions does not expose Journal history. [export][journal] [framework][journaling] | App unlock; AX/file access | M; app sync 1 if enabled | Low UI; high exported-file parsing | No | — | 2 |
| P23 Journal entry creation/search | “Write today's reflection.” | R search results; W text/audio entry | Phone Journal actions documented; Mac equivalents/output fields unverified. [actions][sc18] | Journal/Shortcut grants; microphone for audio | 3; app sync 1 | Low relay | No | — | 2 |
| P24 Books library/open | “Find my unread books”; “Open this audiobook.” | R discoverable library metadata; W open/play | Books Shortcuts Find Books/Open Book/Play Audiobook; not proof of full reading-state fields. [actions][sc16] | Shortcut/Books access | M / 3; library 1 | Medium; read-status unverified | No | — | 2 |
| P25 Books highlights/progress | “Collect my book highlights.” | R annotations/progress | Candidate local Books SQLite/AX; exact paths and current schema **unverified**, prefer user export. No public Books-library API established. [available actions][sc16] | FDA/AX; user export | M; 1 if synced | Low | No | — | 2 |
| P26 Books protected text | “Export my purchased book as plain text.” | R only user-exportable/unprotected content | File access is not DRM decryption; no supported API for arbitrary protected Books content. [Books action scope][sc16] | File access; license restrictions | M | Impossible blanket DRM export | No | — | 1 |
| P27 Notes checklists/files/folders | “Append a checklist item”; “Attach this PDF.” | W supported structured additions/folder rename | Shortcuts actions documented across platforms in 18.5/15.5; preserve existing content and prove effects. [actions][sc18] | Notes/Shortcut; file access | 1 / 3 | High action; unverified integration | No | — | 3 |

## Media

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| M01 Photos search | “Show photos from Paris last May.” | R assets/date/location/type | `PHAsset` fetch; semantic people/scene search is not a general PhotoKit query. [PhotoKit][photo] | Photos read | 1 | High | No | [osxphotos][osxphotos] | 3 |
| M02 Photos originals | “Download the original video.” | R image/video/Live Photo resources | `PHAssetResourceManager`, network-enabled retrieval for iCloud assets. [PhotoKit][photo] | Photos read; network | 1 | High | No | [osxphotos][osxphotos] | 3 |
| M03 Photos import | “Save these pictures to Photos.” | W image/video assets | `PHPhotoLibrary.performChanges`, asset creation. [PhotoKit][photo] | Photos add/read-write as needed | 1 | High | No | [osxphotos][osxphotos] | 3 |
| M04 Photos albums | “Create a Vacation album.” | R/W albums/membership | `PHAssetCollectionChangeRequest`; respect immutable/system albums. [PhotoKit][photo] | Photos read-write | 1 | High | No | [osxphotos][osxphotos] | 3 |
| M05 Photos favorite/metadata/delete | “Favorite these”; “Correct this date.” | W allowed asset fields/deletions | `PHAssetChangeRequest`; deletion may require confirmation; not face labeling. [PhotoKit][photo] | Photos read-write; system confirmation | 1 | High | No | [osxphotos][osxphotos] | 2 |
| M06 Photos people/keywords | “Find all pictures of Alice.” | R locally recognized metadata | `~/Pictures/Photos Library.photoslibrary/database/Photos.sqlite` by default; library selectable; read-only parser. [osxphotos][osxphotos] | FDA/library file access | M; locally available 1 data | Medium | No | [osxphotos][osxphotos] | 3 |
| M07 Photos image edits | “Crop this”; “Make a brighter copy.” | R pixels; W derived asset/non-destructive adjustment | PhotoKit content editing + Core Image; own edits, not arbitrary manipulation of Photos' private edit stack. [PhotoKit][photo] | Photos read-write | 1 | High | No | [osxphotos][osxphotos] (export reference) | 2 |
| M08 Music now playing | “What song is playing?” | R Music app track/player state | Music AppleScript, not universal system Now Playing. [source][i-music] | AE Music | M; phone query requires 3 | Medium | `music_now_playing` [source][i-music] | — | 3 |
| M09 Music playback | “Pause”; “Skip”; “Seek to one minute.” | W Music transport | Music AppleScript play/pause/next/previous/stop/seek. [source][i-music] | AE Music | M; 3 for iPhone player | Medium | `music_control` [source][i-music] | [Hammerspoon][hs] | 3 |
| M10 Music catalog | “Find albums by this artist.” | R catalog, artwork | MusicKit search; developer capability and account/subscription conditions depend on operation. [source][i-music] [MusicKit][music] | Music authorization/capability | M, results usable anywhere | High | `music_catalog_search` [source][i-music] | — | 3 |
| M11 Music library/playlists | “Create a running playlist.” | R/W supported library items/playlists | MusicKit library APIs where available; Music AppleScript for local library operations. [MusicKit][music] [scripting][sb] | Music authorization or AE | 1 with Sync Library; playback not synced control | Medium overall | No | — | 3 |
| M12 Music ratings/favorites | “Love this track”; “Set five stars.” | W supported rating/favorite metadata | Music AppleScript dictionary; fields and cloud propagation require build probe. [scripting][sb] [player implementation][i-music] | AE Music | M; 1 propagation unverified | Medium, unverified fields | No | — | 2 |
| M13 Podcasts discover/play | “Play the newest episode.” | R episode metadata; W playback | Podcasts Shortcuts/app UI candidate; exact desktop actions unverified; public RSS is an alternative, not Apple library access. [Shortcuts model][sc-cli] | Shortcut/app; AX fallback | M / 3 | Low pending action probe | No | — | 2 |
| M14 Podcasts follows/progress | “What episodes have I not finished?” | R/W library/progress | No public personal Podcasts library API found; AX/local-store investigation unverified; do not invent SQLite path. [generic AX][ax] | FDA/AX if implemented | M; 1 app sync conditional | Low | No | — | 2 |
| M15 TV library/playback | “Play my downloaded film.” | R metadata; W play/pause | TV scripting dictionary if exposed; AX fallback; current dictionary unverified. DRM remains intact. [ScriptingBridge][sb] | AE TV/AX | M; 3 action availability unverified | Low | No | — | 2 |
| M16 TV watch history/Up Next | “What's next in my shows?” | R personal TV state | No public account-wide API found; visible UI extraction unverified. [AX][ax] | AX | M; 1 app data | Low | No | — | 2 |
| M17 Voice Memos list | “Find yesterday's recording.” | R title/date/duration | `~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db`. [source][i-voice] | FDA | 1 | Medium | `voice_memos_fetch` [source][i-voice] | [TSRP][tsrp] | 3 |
| M18 Voice Memos transcript | “Summarize my recorded meeting.” | R embedded transcript if local | Parse transcript from `.m4a`/`.qta`; don't assume every recording has one. [source][i-voice] | FDA | 1 | Medium | `voice_memo_transcript_fetch` [source][i-voice] | [TSRP][tsrp] | 3 |
| M19 Voice Memos audio | “Export that memo.” | R recording bytes | Adjacent recordings, metadata-resolved file access. [source][i-voice] | FDA | 1 | Medium | `voice_memo_audio_fetch` [source][i-voice] | [TSRP][tsrp] | 3 |
| M20 Voice Memos create/manage | “Record a memo”; “Delete the test memo.” | W recording/folder/delete | Shortcuts actions; platform/foreground requirements need probe; AVFoundation recording alone does not add to Voice Memos. [actions][sc16] | Microphone; Shortcut/app approval | M action availability unverified; 3 | Low route | No | — | 2 |
| M21 Transcribe arbitrary audio | “Transcribe this interview.” | R audio; W bridge transcript | Speech framework or Shortcuts transcription; language/on-device support varies. [Speech][speech] | Speech Recognition where required; mic only for live input | M | High | No | [TSRP][tsrp] (existing transcripts only) | 3 |

## Location and travel

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| L01 Current Mac location | “Where is my Mac?” | R coordinate/accuracy/time | CoreLocation; not guaranteed GPS accuracy. [source][i-location] | Location Services | M | High | `location_current` [source][i-location] | — | 3 |
| L02 Geocoding | “Locate this address”; “What address is this?” | R address-coordinate conversion | `CLGeocoder`/current MapKit geocoding APIs, rate limited. [source][i-location] | None for supplied coordinates; network | M | High | `location_geocode`, `location_reverse-geocode` [source][i-location] | — | 3 |
| L03 Nearby places | “Find a pharmacy near me.” | R POIs | `MKLocalSearch`. [source][i-maps] | Location only if resolving current position | M | High | `maps_search`, `maps_explore` [source][i-maps] | — | 3 |
| L04 Routes and ETA | “How long to the airport?” | R route steps/distance/time | `MKDirections`; supported transport modes and coverage only. [source][i-maps] | None for supplied endpoints; location optional | M; 3 for phone's origin | High local | `maps_directions`, `maps_eta` [source][i-maps] | — | 3 |
| L05 Map image | “Show a map of the route.” | R generated map snapshot | `MKMapSnapshotter`; preserve attribution. [source][i-maps] | Network | M | High | `maps_generate` [source][i-maps] | — | 2 |
| L06 Launch navigation | “Navigate home on my phone.” | W Maps directions UI | Mac `MKMapItem.openInMaps` only opens Mac; iPhone needs route 3/4 foreground launch. [MapKit][maps] | Location for navigation; phone UI | M / 3 / 4 | High Mac launch; low remote route | No dedicated tool | — | 3 |
| L07 Maps favorites/guides | “Save this place to my Travel guide.” | R/W personal saved places | No public account favorites/guide CRUD API found; Maps AX candidate unverified. [MapKit scope][maps] | AX | 1 if app sync | Low | No | — | 2 |
| L08 Weather now | “Do I need a coat?” | R current conditions | WeatherKit; entitlement, network, required attribution. [source][i-weather] | WeatherKit capability; location optional | M; coordinate anywhere | High conditional capability | `weather_current` [source][i-weather] | — | 3 |
| L09 Weather forecast | “Will it rain tonight?” | R hourly/daily/next-hour precipitation | WeatherKit; minute forecast is region dependent. [source][i-weather] | WeatherKit capability | M | High conditional capability | `weather_daily`, `weather_hourly`, `weather_minute` [source][i-weather] | — | 3 |
| L10 Weather alerts/saved locations | “Warn me about storms”; “Save Boston.” | R alerts; W saved-place preference | WeatherKit alerts; Shortcuts saved-location actions separately. [WeatherKit][weather] [actions][sc26] | WeatherKit; notification permission for bridge alerts | M; 1/3 saved-list behavior unverified | High alerts; medium preference candidate | No dedicated tools | — | 2 |
| L11 Current phone location | “Where is my phone right now?” | R phone location | No Mac CoreLocation remote query. Phone Get Current Location via 3, or companion CLLocationManager via 4. [CoreLocation][location] | Phone location grant; background grant if needed | 3 / 4 | Low relay; high API, conditional companion | No | — | 3 |
| L12 Find My devices/items/people | “Where are my keys?” | R visible Find My data | No public general Find My account API; Mac AX candidate unverified. Private web protocols are out of policy. [Find My guide][findmy] | Signed-in Find My; AX | M UI reflects Apple service; not route 1 file sync | Low | No | [pyicloud][pyicloud], [OpenHaystack][haystack] (private/protocol research, not approved backend) | 3 |
| L13 Find My play sound | “Make my lost phone ring.” | W Find My sound request | Find My UI AX proposal; a route 3 sound is only an online approximation, not offline finding. [Apple guide][findmy] | Signed-in app; AX; device reachable eventually | M → Find My service; 3 approximation | Low | No | [pyicloud][pyicloud] private API, out of policy | 3 |
| L14 Lost Mode/erase | “Mark the phone lost.” | W account/device security action | Find My owner UI only; no supported general programmatic API found; AX unverified, no unattended erase proposal. [Find My][findmy] | Account authentication/confirmation | Find My service | Low assisted; impossible public API | No | [pyicloud][pyicloud] out of policy | 2 |
| L15 Parked car | “Where did I park?” | R/W parked-car marker | Phone Shortcuts parked-car actions. [actions][sc16] | Location/Maps approval | 3 | Low route | No | — | 2 |
| L16 Arrival/departure events | “Tell me when I get home.” | R geofence events; W bridge automation | Companion CoreLocation monitoring, or phone personal automation; not Find My surveillance. [CoreLocation][location] [automation][auto] | Phone location/background authorization | 3 / 4 | Low relay; high API conditional companion | No | — | 3 |

## Home and devices

HomeKit is **not available to the current native macOS Helper**. Direct framework access requires a separately provisioned Catalyst helper or iPhone companion. For a narrower control surface, macOS Shortcuts can execute Home actions without giving the native Helper HomeKit entitlement; Apple documents Mac accessory/scene toggling (H16). [Shortcuts 18/15][sc18] Public API quality does not remove the entitlement/distribution gate: verify signing/distribution before committing to a product implementation. iMCP's source is unusually broad here, including administration and automation tools. Its supported automation subset is not the entirety of Apple's Home app automation representation. [Apple HomeKit platforms][home] [iMCP deployment limitation][imcp] [tool definitions][i-home]

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| H01 Home inventory/export | “List my homes and smart devices.” | R homes, accessories, services, metadata | HomeKit through Catalyst IPC, not native Helper. [source][i-home] | HomeKit entitlement + Home approval | H; same Home visible on phone; 4 alternative | High API, distribution gate | `homes_list`, `home_export`, `accessories_list`, `accessories_get` [source][i-home] | — | 3 |
| H02 Rooms/zones list | “What's in the kitchen?” | R rooms/zones/membership | HomeKit Catalyst. [source][i-home] | Home grant/entitlement | H / 4 | High conditional | `rooms_list`, `zones_list` [source][i-home] | — | 2 |
| H03 Sensors/state | “Are the doors closed?” | R readable characteristics | `HMCharacteristic.readValue`; reachability and accessory support matter. [source][i-home] | Home grant/entitlement | H / 4 | High conditional | `characteristics_read` [source][i-home] | — | 3 |
| H04 Lights/plugs/thermostats | “Dim the lights”; “Set 20°C.” | W writable characteristics | HomeKit characteristic type/range checks. [source][i-home] | Home grant/entitlement | H / 4; 3 Home action candidate | High conditional; 3 low | `characteristics_write` [source][i-home] | — | 3 |
| H05 Locks/garage/security | “Lock the front door.” | R/W permitted security characteristics | Same API, accessory/remote-access and security restrictions apply; locked-phone Shortcut behavior unverified. [source][i-home] | Home authorization; possible authentication | H / 4 / 3 | High API conditional; low relay | `characteristics_read`, `characteristics_write` [source][i-home] | — | 3 |
| H06 Scene list/execute | “Run Good Night.” | R scenes; W execute | `HMHome.executeActionSet`. [source][i-home] | Home grant/entitlement | H / 4 / 3 | High conditional; 3 low | `scenes_list`, `scenes_execute` [source][i-home] | — | 3 |
| H07 Scene CRUD | “Make a Movie Night scene.” | W scene/action sets | HomeKit action-set CRUD, only supported characteristics. [source][i-home] | Home grant/entitlement + edit rights | H / 4 | High conditional | `scenes_create`, `scenes_update`, `scenes_remove` [source][i-home] | — | 2 |
| H08 Home automations | “Turn the light on at sunset.” | R/W supported triggers and enablement | HomeKit timer/characteristic/sun triggers; Home-app Shortcut internals and last-fire time not complete. [source][i-home] | Home edit rights/entitlement; hub as needed | H / 4 | High conditional, partial model | `automations_list`, `automations_create`, `automations_update`, `automations_remove` [source][i-home] | — | 3 |
| H09 Room/zone CRUD | “Move the lamp to Office.” | W grouping/topology | HomeKit room/zone updates. [source][i-home] | Home edit rights/entitlement | H / 4 | High conditional | `rooms_create`, `rooms_rename`, `rooms_remove`, `zones_create`, `zones_update`, `zones_remove`, `accessories_assign_room` [source][i-home] | — | 2 |
| H10 Accessory administration | “Rename this lamp”; “Identify it.” | W names/identify/removal | HomeKit; removing may require repairing later. [source][i-home] | Home edit rights/entitlement | H / 4 | High conditional | `accessories_identify`, `accessories_rename`, `services_rename`, `accessories_remove` [source][i-home] | — | 2 |
| H11 Home cameras | “Show the doorbell camera.” | R supported camera stream/snapshot | HomeKit camera APIs via entitled helper; no general HomeKit Secure Video archive export API established. [HomeKit][home] | Home/camera-access rights; entitlement | H / 4 | High API conditional; archive unverified | No dedicated tool | — | 2 |
| H12 AirPlay output | “Play this on the living-room speaker.” | R route picker; W chosen route | `AVRoutePickerView` for app media; Music dictionary/Shortcuts route actions are separate and device-dependent. [API][airplay] | Local Network where needed; UI/AE | M → receiver; 3 phone player | Medium; arbitrary headless routing unverified | No | [Hammerspoon][hs] (audio devices) | 3 |
| H13 Bluetooth inventory/connect | “Which headphones are connected?” | R paired devices/state; W supported connection | Public IOBluetooth device APIs, CoreBluetooth for app-owned BLE; not universal radio power. [IOBluetooth][bluetooth] [CoreBluetooth][ble] | Bluetooth; Local Network unrelated | M | High for supported API/device | No | [blueutil][blueutil] (mixed public/private; audit calls) | 2 |
| H14 Bluetooth battery | “What's my headset battery?” | R exposed battery values | BLE Battery Service if exposed; AirPods vendor/system metadata route unverified. [CoreBluetooth][ble] | Bluetooth | M; 3 phone specifics unverified | High standard BLE; low AirPods-general | No | [blueutil][blueutil] (inventory only) | 2 |
| H15 Bluetooth radio power | “Turn Bluetooth off on the Mac.” | W radio state | Shortcut if available, else AX; `IOBluetoothPreference*` used by blueutil is private and out of policy. [source warning][blueutil] | Shortcut approval/AX | M | Low until supported action proven | No | [blueutil][blueutil] excluded private path | 2 |
| H16 Home control through Mac Shortcuts | “Toggle the bedroom lamp.” | W selected accessory/scene | Mac Home action documented in 15.2; Shortcut runs inside Apple's privileged integration, not native HomeKit linkage. [actions][sc18] | Home membership + Shortcut permission | M → H; state visible on phone | High documented action; device behavior unverified | No dedicated tool; `shortcuts_run` indirectly | — | 3 |

## Device control on the Mac

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| D01 Volume/mute | “Mute my Mac”; “Set volume to 30%.” | R/W output volume/mute | CoreAudio device properties, or AppleScript `get volume settings`/`set volume`. [CoreAudio][audio] | None for normal output control | M | High API; medium OSA | No | [Hammerspoon audio][hs-audio] | 3 |
| D02 Audio input/output device | “Use my USB microphone.” | R devices; W default route/input gain | CoreAudio supported properties; virtual devices may differ. [CoreAudio][audio] | Microphone only for recording, not mere selection | M | High | No | [Hammerspoon audio][hs-audio] | 2 |
| D03 Brightness | “Dim my display.” | R/W display brightness | Shortcuts/AX preferred, exact action support unverified; modern `brightness` uses undocumented APIs, out of policy. [project warning][brightness] | Shortcut approval/AX | M | Low until supported path verified | No | [brightness][brightness] excluded private paths | 3 |
| D04 Dark mode | “Switch to dark mode.” | R/W system appearance | System Events appearance preferences AppleScript; probe dictionary. [ScriptingBridge][sb] [automation reference][hs] | AE System Events | M | Medium | No | [Hammerspoon][hs] | 2 |
| D05 Night Shift/True Tone | “Enable Night Shift.” | W supported display setting | Shortcuts documented actions; avoid private CoreBrightness APIs. [actions][sc16] | Shortcut approval; compatible display | M | High action; current hardware unverified | No | — | 2 |
| D06 Wi-Fi state/join | “Join my home Wi-Fi.” | R interface/SSID; W power/association | CoreWLAN; network credentials required; SSID/BSSID can require location authorization. [CoreWLAN][wifi] | Location for protected network details; admin authorization for some changes | M | High API, authorization conditional | No | [Hammerspoon Wi-Fi][hs-wifi] | 3 |
| D07 Network diagnostics | “Why am I offline?” | R reachability/interfaces/DNS | Network framework path monitor/SystemConfiguration; not Internet success merely because interface is up. [Network][network] | Usually none; Local Network for probes | M | High | No | [Hammerspoon][hs] | 3 |
| D08 Launch/quit apps | “Open Pages”; “Quit Music.” | R running apps; W launch/terminate | `NSWorkspace`, `NSRunningApplication`; unsaved-document prompts remain. [API][workspace] | Usually none; AE for app commands | M | High | No | [Hammerspoon][hs] | 3 |
| D09 Windows | “Put Safari on the left.” | R/W visible window frames/focus | AXUIElement or supported Shortcuts window actions; app must expose attributes. [AX][ax] [actions][sc17] | AX, or Shortcut approval | M | Low AX; high documented action when suitable | No | [Hammerspoon][hs] | 3 |
| D10 UI buttons/menus | “Click Export in this app.” | R AX tree; W UI actions | AXUIElement; not a semantic app API, locked session/secure fields remain boundaries. [AX][ax] | AX; AE if System Events | M | Low | No | [Hammerspoon][hs], [Beeper][beeper] | 3 |
| D11 Clipboard | “Copy this”; “What's on my clipboard?” | R/W general pasteboard | `NSPasteboard`; Universal Clipboard is proximity-dependent, not a remote sync bus. [API][clipboard] [Continuity][universal] | Pasteboard consent where OS requires; no FDA prerequisite | M; 2 nearby clipboard handoff | High local | No | [Hammerspoon][hs] | 3 |
| D12 Screenshot | “Show me the screen.” | R captured pixels | ScreenCaptureKit `SCScreenshotManager`; selected display/window. [source][i-capture] | Screen Recording | M | High | `capture_take_screenshot` [source][i-capture] | [Hammerspoon][hs] | 3 |
| D13 Screen video/system audio | “Record this demo.” | R capture stream; W recording file | ScreenCaptureKit; protected media can be excluded. [framework][capture] | Screen & System Audio Recording; mic separately | M | High | No (screenshot only) | — | 2 |
| D14 Camera still | “Take a webcam photo.” | R camera frame; W image | AVFoundation capture session. [source][i-capture] | Camera | M; Continuity Camera only if configured | High | `capture_take_picture` [source][i-capture] | — | 2 |
| D15 Microphone recording | “Record a voice note on the Mac.” | R microphone; W audio file | AVFoundation; not insertion into Voice Memos library. [source][i-capture] | Microphone | M | High | `capture_record_audio` [source][i-capture] | — | 2 |
| D16 Play sound | “Ping my Mac.” | W audible beep | AppKit `NSBeep`/`NSSound`; muted/output-device settings still apply. [source][i-utils] | None | M | High | `utilities_beep` [source][i-utils] | [Hammerspoon][hs] | 2 |
| D17 Bridge notifications | “Notify me when the upload finishes.” | W own app notification | UserNotifications; authorized bridge notifications, not other apps' notification databases. [framework][notifications] | Notifications | M; 4 APNs for phone | High | No | [Hammerspoon][hs] | 3 |
| D18 Other apps' notifications | “Read my recent alerts.” | R visible notifications or permitted trigger input | No general `UNUserNotificationCenter` cross-app history API. AX is unverified; OS 27 notification automation is a candidate, not proof of full contents/history. [notifications][notifications] [27 actions][sc27] | AX or per-automation permission | M / 3 | Low | No | — | 3 |
| D19 Battery/power state | “How much battery does the Mac have?” | R charge/source/time estimates | IOKit power sources or `pmset -g batt`; supported hardware fields only. [Apple pmset man][pmset] | None for reads | M | High | No | [Hammerspoon battery][hs-battery] | 3 |
| D20 Sleep/keep awake | “Keep the Mac awake for an hour.” | W sleep assertion/sleep request | `caffeinate`/IOPM assertions; `pmset` for allowed sleep operations. [Apple man][caffeinate] [pmset][pmset] | None for assertions; admin for some settings | M | High | No | [Hammerspoon][hs] | 3 |
| D21 Lock/restart/shutdown | “Lock the Mac”; “Restart tonight.” | W session/system action | Shortcuts actions; confirmations/unsaved work may block. [actions][sc16] | Session rights; confirmation/admin as applicable | M | High action, unattended completion conditional | No | [Hammerspoon][hs] | 2 |
| D22 Wallpaper | “Use this image as my desktop.” | R/W per-display desktop image | `NSWorkspace.setDesktopImageURL`. [API][wallpaper] | Read access to image | M | High | No | [Hammerspoon][hs] | 2 |
| D23 System Settings navigation | “Open Privacy settings.” | W opens settings pane | `NSWorkspace` app open is public; individual `x-apple.systempreferences:` routes are undocumented/unverified. [workspace][workspace] | None to open; AX to manipulate | M | High app open; low pane automation | No | — | 2 |
| D24 TCC/security settings | “Grant yourself camera access.” | R current status; W only OS permission request | Public request APIs can prompt; no API to silently self-grant TCC. Direct TCC.db writes/private bypasses excluded. [AVFoundation][av] [AX trust][ax-trust] | Owner/system authorization | M | Impossible silent grant | No | — | 1 |
| D25 VPN | “Connect my work VPN.” | R/W supported VPN connection | Shortcuts Set VPN; installed profile credentials/authorization still required. [actions][sc16] | VPN configuration rights; system approval | M / 3 | High local action; low relay | No | — | 2 |
| D26 Low Power/charge limit on Mac | “Limit charging”; “Is Low Power on?” | R Low Power; W supported charge limit | `ProcessInfo.isLowPowerModeEnabled`; Shortcuts charge-limit action on compatible hardware; system Low Power writes via UI/action unverified. [API][lowpower] [26.4 action][sc26] | Shortcut approval/admin if system change demands | M | High read/action; low unverified setting | No | — | 2 |

## Focus, Do Not Disturb and Screen Time

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| F01 Focus status | “Are notifications silenced?” | R shared silenced state | `INFocusStatusCenter`; not complete Focus identity/settings or another person's private status. [API][focus-api] | Focus status authorization/usage description | M; 1 status reflects shared Focus configuration | High | No | — | 3 |
| F02 Current Focus name | “Am I in Work Focus?” | R selected Focus | Shortcuts Get Current Focus; native Focus-status API alone does not supply full configuration. [actions][sc16] | Shortcut approval | M / 3 | High local action; low relay | No | — | 3 |
| F03 Set Focus/DND | “Do Not Disturb for an hour.” | W selected Focus/on/off/duration | Installed Set Focus Shortcut candidate; Control Center AX fallback. Apple documents cross-device on/off sync. [Apple behavior][focus-sync] | Shortcut approval/AX; Share Across Devices enabled | M → 1, or 3 | Medium pending action probe; low AX/relay | No | — | 3 |
| F04 Focus rules/schedules | “Allow only family during Work.” | R/W UI configuration | System Settings AX; no general public Focus CRUD framework identified. [Apple settings][focus-settings] | AX; account configuration | 1 | Low | No | — | 2 |
| F05 App Focus filters | “Show only my work account in the bridge.” | R/W bridge's own filter state | App Intents Focus filter integration; not permission to reconfigure every app. [App Intents][intents] | App configuration/Focus selection | M / 4 | High for own app | No | — | 2 |
| F06 Usage estimates, baseline | “How long did I use Instagram?” | R per-app/device/day estimates | Existing Biome `~/Library/Biome/streams/restricted/App.InFocus/{local,remote}` parser; incomplete sync/intervals remain estimates. [repo contract](../design/SPEC.md#96-usage-screen-time-read-only) | FDA | M; 1 received remote streams | Medium | No | Existing bridge decoder | 3 |
| F07 Public Screen Time action | “Show my app and website usage.” | R action-defined usage data | OS 26 Shortcuts “Get App & Website Data”; output schema, detail, permissions and unattended behavior **unverified**. [Apple announcement][sc26] | Screen Time/Shortcut authorization, exact prompts unverified | M / 3 | Medium candidate; low until output proven | No | — | 3 |
| F08 iPhone app limits/shields | “Block social apps after 30 minutes.” | W companion-owned restrictions | No native Mac remote API. Phone FamilyControls authorization + ManagedSettings + DeviceActivity extensions. Not editing Apple's existing App Limits entries. [frameworks][family] [settings][managed] [activity][activity] | Approved Family Controls entitlement; owner/guardian authorization | 4 | High APIs, substantial entitlement/companion gate | No | — | 3 |
| F09 Downtime-like schedule | “Block distracting apps from 10 pm.” | W own scheduled shields | Companion DeviceActivity schedule invokes own ManagedSettings restrictions; not Apple Downtime CRUD. [activity][activity] [managed][managed] | Same Family Controls grant/extensions | 4 | High conditional | No | — | 3 |
| F10 Apple's existing limits/downtime | “Change my existing Screen Time downtime.” | R/W Apple Settings UI | No supported general API to read/write those settings; Mac AX proposal may sync if configured, exact behavior unverified. FamilyControls is not that API. [framework scope][family] | AX; Screen Time passcode/owner approval where required | M; 1 propagation unverified | Low assisted; impossible supported headless API | No | — | 2 |
| F11 Detailed usage replication via companion | “Export every app's Screen Time history.” | R only permitted report data | DeviceActivity reporting is privacy-constrained; no blanket unrestricted export promise. OS 26 action needs evaluation separately (F07). [DeviceActivity][activity] | Family Controls authorization/entitlement | 4 | Low for exhaustive remote export, unverified | No | — | 3 |

## Clock: Mac and iPhone alarms and timers

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| K01 Mac Clock alarms list/create/toggle | “Set a Mac alarm for 8”; “List alarms.” | R alarms; W create/enable | `shortcuts run` installed Clock workflow; documented since macOS 13. No native Clock alarm framework API needed. [actions][sc16] [CLI][sc-cli] | Shortcut/Clock grants | M; does not sync alarm to iPhone | High documented action | No dedicated tool; runner only [source][i-shortcuts] | — | 3 |
| K02 Mac alarm delete | “Delete tomorrow's Mac alarm.” | W removal | Delete Alarms action exists; exact Mac support/confirmation behavior needs probe, AX fallback. [actions][sc17] | Shortcut/Clock grant; AX fallback | M | Medium candidate | No | — | 2 |
| K03 Mac Clock timer | “Start a ten-minute timer here.” | W timer | Shortcuts Start Timer documented on Mac; completion relies on Mac power/session state. [actions][sc16] | Shortcut/Clock grant | M | High action | No | — | 3 |
| K04 Mac timer inspect/pause/cancel | “How much time remains?” | R/W timer state | iOS timer actions documented; Mac availability unverified, Clock AX fallback. [platform distinction][sc17] | Shortcut/AX | M | Low until supported Mac action proven | No | — | 2 |
| K05 iPhone Clock alarms | “Wake me at 7”; “Disable the weekday alarm.” | R/W existing Clock alarms | Route 3 Clock Find/Create/Toggle/Delete actions; no iCloud alarm mirroring from Mac. [Clock actions][sc16] [updated actions][sc17] | Phone Shortcut/Clock permission; deletion may prompt | 3 | Low relay | No | — | 3 |
| K06 iPhone Clock timers | “Set a pasta timer”; “Pause my timer.” | R current timer; W start/pause/resume/cancel | Phone Clock actions through router; exact multi-timer targeting unverified. [actions][sc17] | Phone Shortcut permission | 3 | Low relay | No | — | 3 |
| K07 Companion alarms/countdowns | “Schedule a reliable bridge alarm.” | R/W own app alarms/timers | AlarmKit on iOS 26+; own-client alarms only, not Apple's Clock inventory. Native macOS unavailable; Catalyst is listed but runtime support unverified here. [AlarmKit][alarm] [manager][alarm-manager] | AlarmKit authorization/usage description | 4 | High API, companion/delivery gate | No | [Apple sample][alarm-sample] | 3 |
| K08 Companion stop/snooze/repeat | “Snooze the bridge alarm.” | R/W own alarm lifecycle | AlarmManager stop/countdown/pause/resume/cancel and scheduling; no cross-app alarm privileges. [manager][alarm-manager] | Alarm authorization | 4 | High conditional | No | [Apple sample][alarm-sample] | 3 |
| K09 Sleep wake-up exception | “Skip tomorrow's sleep alarm.” | W skip/reactivate next sleep alarm | Phone Edit Sleep Alarm action; not general sleep-schedule editing. [actions][sc17] | Phone Shortcut permission | 3 | Low relay | No | — | 3 |
| K10 Stopwatch | “Start a stopwatch on my phone.” | W start/lap/stop/reset | Phone Clock Shortcuts actions. [actions][sc17] | Phone Shortcut permission | 3 | Low relay | No | — | 1 |
| K11 Bridge-only timer fallback | “Notify me in ten minutes.” | W bridge timer/own notification | Dispatch scheduler + UserNotifications; not Clock and cannot promise wake-from-power-off. [notifications][notifications] | Notifications; awake/reliable scheduler | M; 4 notification | High local | No | — | 2 |
| K12 World Clock cities | “Add Tokyo to World Clock.” | W city list | Shortcuts Add City/Remove City documented across platforms; list sync behavior unverified. [actions][sc18] | Clock/Shortcut permission | M / 3 | High local action; low relay | No | — | 1 |

## Shortcuts and its wider reach

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| S01 List shortcuts/folders | “What automations can you run?” | R installed names/identifiers/folders | `/usr/bin/shortcuts list`, folder options. [CLI][sc-cli] [implementation][i-shortcuts] | Logged-in user's library access | M; 1 library sync conditional | High | `shortcuts_list` [source][i-shortcuts] | — | 3 |
| S02 Run with typed I/O | “Run my meeting-prep Shortcut.” | R input/output; W action effects | `shortcuts run` with input/output paths/type; may pause for UI; success ≠ downstream sync. [CLI][sc-cli] | Every action's own permissions | M; 1/2 only if action does it | High runner | `shortcuts_run` [source][i-shortcuts] | — | 3 |
| S03 Router on iPhone | “Run my phone routine remotely.” | R command/result; W dispatched action | Existing Messages send + receiver automation + Run Shortcut; **whole round trip unverified**. [trigger][message-trigger] [automatic execution][auto] | Owner setup; Messages/each target action grants | 3 | Low | No | [imsg][imsg-art] (transport, not router implementation) | 3 |
| S04 Discover app actions | “Can this app export invoices?” | R installed action choices, not guaranteed CLI schema enumeration | App Intents supply actions; browse Shortcuts editor. No public CLI inventory of every action/schema established. [App Intents][intents] | App installed; editor UI if inspected | M / 3 / 4 | High integration; low automated discovery | No | [osa-mcp][osa-discover] (OSA dictionaries, not App Intents) | 2 |
| S05 Provision/share shortcuts | “Install my bridge workflow.” | W user-imported signed workflow | `shortcuts sign`/view and explicit import; not permission to silently enroll all personal automations. [CLI][sc-cli] | User import/action grants | M; 1 library, 3 provisioning separately | Medium deployment | No | [Shortcuts format research][sc-format] (historical/undocumented) | 3 |
| S06 Scheduled/event workflows | “When a file arrives, process it.” | R trigger; W routed actions | macOS 26 personal automations; also bridge-owned scheduler/FSEvents. [release notes][sc26] | Per-action/trigger grants | M; 3 phone triggers | High documented local behavior; configuration unverified | No | [Hammerspoon][hs] | 3 |
| S07 File/PDF/image transforms | “Resize these”; “Combine PDFs.” | R inputs; W derived files | Shortcuts actions or native PDFKit/Vision/Core Image; no arbitrary app-store access implied. [PDFKit][pdf] [Vision][vision] | File access | M / 3 | High local | No | [orchard-mcp][orchard] | 3 |
| S08 HTTP/webhook integration | “Send this result to my workflow.” | R/W explicitly configured HTTP endpoint | Shortcut Get Contents of URL or URLSession in Helper; external service authorization separate. [documented action][sc16] | Network; endpoint authorization | M / 3 / 4 | High local; low relay | No | — | 2 |
| S09 27 workflow state/notifications | “Remember the last run”; “React to an alert.” | R/W workflow state/trigger input | OS 27 adds storage and notification triggers; input fields and device filtering unverified. [release notes][sc27] | Shortcut/notification-trigger permissions | M / 3 | Medium candidate | No | — | 2 |
| S10 27 Messages alternatives | “React without UI scripting.” | W supported Messages actions | OS 27 documents additional actions; exact GUID targeting unverified; evaluate as baseline replacement only after real proof. [release notes][sc27] | Messages/Shortcut permission | M / 3 | Medium candidate | No dedicated tools beyond reads | [Beeper][beeper], [imsg][imsg-art] (existing mechanisms) | 3 |

## Health and fitness

The Mac cannot read or write the iPhone's Health store through HealthKit or an iCloud database path. Route 3 may handle supported Health actions; route 4 is the durable architecture, with explicit per-type consent and its own authenticated replication. API availability is not evidence of background freshness: Apple limits background delivery, and locked-device protected data can be inaccessible. Request authorization for each data type; a lack of readable samples must not be reported as proof of no health event. [HealthKit architecture][health-about] [Health authorization][health-auth] [background delivery][health-bg]

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| E01 Steps/distance/energy | “How many steps today?” | R quantity samples/aggregates | No Mac store; phone HKStatistics queries or supported Health Shortcut. [HealthKit][health] | Phone Health read per type | 4 / 3 | High phone API; low relay | No | [SpeziHealthKit][spezi] | 3 |
| E02 Heart rate/HRV/vitals | “Show my resting heart-rate trend.” | R authorized samples, not live Watch stream by default | Phone HKSampleQuery/statistics; sensor/type availability varies. [HealthKit][health] | Per-type Health consent | 4 / 3 if action supports type | High phone API conditional | No | [SpeziHealthKit][spezi] | 3 |
| E03 Sleep | “How much deep sleep did I get?” | R category intervals | Phone HealthKit sleep-analysis samples; sleep stages via supported Shortcuts. [HealthKit][health] [actions][sc16] | Health sleep read | 4 / 3 | High phone API; low relay | No | [SpeziHealthKit][spezi] | 3 |
| E04 Weight/body measurements | “Log 74 kg”; “Show my trend.” | R/W supported quantity samples | Phone HealthKit save/query, no native Mac access. [sample model][health-about] | Per-type Health read/write | 4 / 3 | High phone API; low relay | No | [SpeziHealthKit][spezi] | 3 |
| E05 Water/nutrition | “Log a glass of water.” | R/W nutrition quantities/correlations | Phone HealthKit/Log Health Sample; validate units and avoid duplicate saves. [HealthKit][health] | Health per-type read/write | 4 / 3 | High phone API; low relay | No | [SpeziHealthKit][spezi] | 3 |
| E06 Workouts and routes | “List my runs”; “Show this route.” | R workout/route samples; W own workout records | HKWorkout/HKWorkoutRoute queries/builder on phone; route consent separate. [HealthKit][health] | Workout/route read; write if logging | 4 / 3 partial | High phone API conditional | No | [SpeziHealthKit][spezi] | 3 |
| E07 Activity rings | “Did I close my rings?” | R activity summary, not arbitrary ring edits | Phone HKActivitySummaryQuery; no ability to directly set system ring totals. [API][rings] | Health activity-summary permission | 4 | High phone API | No | [SpeziHealthKit][spezi] | 2 |
| E08 Mindfulness/mood | “Log a mindful session”; “Track my mood.” | R/W supported samples | Phone HealthKit state-of-mind/category types; Shortcut type support varies. [HealthKit][health] | Relevant Health read/write | 4 / 3 partial | High phone API conditional | No | [SpeziHealthKit][spezi] | 2 |
| E09 Clinical records/medications | “Show my clinical records”; “Log my medicine.” | R approved clinical data; W only specifically documented medication features | Clinical Health Records entitlement/API is distinct from ordinary samples. Arbitrary Health medication schedule CRUD not established; current medication API/action coverage **unverified**. [HealthKit][health] | Clinical/Health entitlements and per-type consent | 4; no Mac store | High supported clinical reads; low medication candidate | No | [SpeziHealthKit][spezi] (framework integration only) | 3 |
| E10 Health changes/deletions | “Keep my health dashboard current.” | R incremental samples/deletions; W own saved data only | Phone anchored/observer queries; cannot freely delete other apps' samples. [HealthKit][health] [background][health-bg] | Per-type authorization; background delivery entitlement if used | 4 | High API, no realtime guarantee | No | [SpeziHealthKit][spezi] | 3 |
| E11 Health export import | “Analyze my Health export.” | R user-provided archive; W bridge index | Owner exports from Health then supplies file to Mac; snapshot, not ongoing iCloud access. [Apple export][health-export] | User export/file access | Manual transfer, outside 1–4 | High file processing | No | [SpeziHealthKit][spezi] (live alternative) | 2 |
| E12 Start/end Watch workout | “Start a run on my watch.” | W supported workout session | Phone/Watch Shortcut or companion Watch app; Mac cannot directly control Apple's Workout app through HealthKit. [Shortcuts actions][sc17] [HealthKit][health] | Watch/workout authorization | 3 / 4 with Watch component | Low complete route, unverified start path | No | — | 2 |

## Wallet and Passwords

Neither is an unrestricted personal-data API. There are useful exceptions, so “Wallet impossible” and “Passwords impossible” must be qualified: authorized issuer passes, user-exported password CSVs, app-owned keychain items, and entitlement-gated FinanceKit are separate capabilities. None grants a general Wallet dump or a general Passwords vault API. [Pass entitlements][wallet-config] [Keychain services][keychain] [Password export][password-export] [FinanceKit][finance]

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| W01 All Wallet cards/passes | “List every card and boarding pass.” | Requested R arbitrary Wallet contents | No supported general access; PKPassLibrary returns only entitled passes, and availability must be checked. [Apple restriction][wallet-config] [pass library][passes] | Issuer/team entitlements cannot be replaced by FDA | No general Mac→phone route | Impossible general API | No | — | 3 |
| W02 Add/manage own passes | “Add this boarding pass.” | W user-approved pass; R/W issuer-authorized passes | Companion PassKit UI; Mac can handle supplied `.pkpass`, not silently enroll arbitrary passes. [Apple guide][wallet-guide] | User add approval; issuer entitlement for management | 4; user opens supplied pass | High permitted subset | No | — | 2 |
| W03 Apple Pay payment credentials | “Export my card number”; “Pay without approval.” | Requested R secret payment material/W unauthorized payment | PassKit does not supply unrestricted payment secrets or waive payment authorization. [PassKit][passkit] | Apple Pay entitlement and transaction authorization | No general route | Impossible stated request | No | — | 1 |
| W04 Transactions/balances | “Show supported account transactions.” | R authorized financial data | FinanceKit on phone; managed entitlement, qualifying organization and supported accounts/regions. No signup undertaken. [requirements][finance] | FinanceKit managed entitlement + user consent | 4 | High API, low eligibility certainty | No | — | 3 |
| W05 Payment event automation | “Log future Wallet purchases.” | R event payload; W own ledger | Phone transaction automation; not historical Wallet API or guarantee of all fields/card coverage. [documented trigger][sc17] | Owner automation setup; payment data access | 3 events → bridge | Low route, payload unverified | No | — | 2 |
| W06 Passwords live vault | “Search all my saved passwords automatically.” | Requested R/W arbitrary Passwords vault | No public general Passwords CRUD API identified; Keychain APIs obey access groups/ACLs and authentication. Show Passwords opens UI only. [Keychain][keychain] [navigation action][sc17] | Unlock/user authentication; FDA is insufficient | No supported general route | Impossible general API; low assisted UI | No | — | 3 |
| W07 User-exported passwords | “Import my exported password CSV.” | R explicit export file; W bridge-controlled processing | Passwords menu export; exclusions apply; not live synchronization and not passkey extraction. [Apple export][password-export] | User export/unlock; file access | M; source vault may sync 1 | High file parse; manual export | No | — | 1 |
| W08 App-owned/ACL-approved secrets | “Use the bridge's saved API credential.” | R/W authorized keychain items | Security `SecItem*`; legacy Mac items may prompt under ACL. No assumption of access to Apple Passwords items. [Keychain][keychain] | Keychain access group/ACL/user authorization | M; 1 only own synchronizable items | High scoped API | No | — | 3 |
| W09 Passkeys/sign-in | “Sign in using my passkey.” | W user-authorized authentication assertion, not key export | AuthenticationServices scoped to relying party; no general private-key extraction. [framework][authservices] | User verification; associated-domain configuration | M / 4; account sync as supported | High authentication; impossible arbitrary export | No | — | 2 |

## Siri and Apple Intelligence

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| A01 Expose bridge actions to Siri | “Siri, run my bridge routine.” | W own app intent | App Intents/App Shortcuts publish bridge actions; not a Siri control API. [framework][intents] | Action-specific grants | M / 4 | High | No | — | 2 |
| A02 Arbitrary Siri RPC/history | “Ask Siri remotely”; “Read Siri's history.” | Requested R/W unrestricted Siri session/history | No public general Siri RPC/history API established; UI invocation is unverified and not dependable structured execution. [App Intents scope][intents] | AX/UI; Siri setup | M; phone route unverified | Impossible general API; low UI | No | — | 2 |
| A03 On-device text model | “Summarize this privately on my Mac.” | R supplied content; W generated response | Foundation Models on supported hardware/OS with model availability checks. It does not automatically inherit all personal data. [framework][models] | Apple Intelligence enabled/eligible hardware; source-data grants separately | M / 4 | High conditional | No | — | 3 |
| A04 Shortcuts model pipeline | “Classify these notes using Apple Intelligence.” | R supplied input; W model output | Use Model action, local/cloud/extension choices; not a blanket Private Cloud Compute REST endpoint. [Apple action][ai-shortcuts] [platforms][sc26] | Apple Intelligence availability; external model consent when applicable | M / 3 | High local action; low relay | No | — | 3 |
| A05 Writing Tools | “Proofread and rewrite this.” | R input text; W transformed text | Shortcuts writing actions; return result rather than silently replace arbitrary app documents. [actions][sc26] | Apple Intelligence availability; source app grants | M / 3 | High local action | No | — | 2 |
| A06 Image Playground | “Create an illustration for this note.” | W generated image | Apple image-generation API/UI or Shortcuts action; supported hardware/content restrictions apply. [actions][sc26] | Apple Intelligence eligibility; user interaction where required | M / 3 / 4 | Medium complete workflow | No | — | 2 |
| A07 OCR/visual understanding | “Read this screenshot”; “Extract the receipt total.” | R image; W OCR/structured result | Vision on Mac; optional model reasoning over authorized result. [Vision][vision] | File access; Screen Recording only for new screen capture | M | High | No | [orchard-mcp][orchard] | 3 |

## iPhone-only settings through the Shortcuts relay

All rows below use **route 3**, so none is a native Mac setting API or a proven background-control channel. A future ordinary companion app does not automatically gain these system-setting privileges. “Candidate” means the named action/variant must be verified in the owner's installed action catalog; the general Shortcuts documentation does not by itself prove every action. Permission means preapproved Shortcut and action-specific access; first-run prompts and locked-device restrictions remain. Prefer explicit set-to-state operations over toggles when retries are possible.

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| I01 Silent mode | “Silence my phone's ringer.” | W silent state; R state availability unverified | Relay → Set Silent Mode candidate; device/model support must be probed, not a universal mute API. [action-discovery model][auto-create] | Phone Shortcut; hardware support | 3 | Low, action/hardware unverified | No | — | 3 |
| I02 Low Power Mode | “Enable Low Power on my phone.” | W Low Power; R where action exposes it | Relay → Set Low Power Mode candidate; native ProcessInfo only reads state. [read-only API][lowpower] [setting triggers][setting-trigger] | Phone Shortcut grant | 3 | Low, exact action probe required | No | — | 3 |
| I03 Battery/charging | “What's my phone battery?” | R level/charging state | Get Battery State/Device Details; return timestamp with result. [documented details][sc16] | Phone Shortcut grant | 3 | Low route | No | — | 3 |
| I04 Battery charge limit | “Stop charging my phone at its supported limit.” | W supported charge limit | OS 26.4 Set Battery Charge Limit; eligible hardware/range unverified. [action][sc26] | Phone Shortcut; supported device | 3 | Low route | No | — | 2 |
| I05 Wi-Fi | “Turn my phone's Wi-Fi on.” | W radio setting; R network details where authorized | Set Wi-Fi candidate, Get Network Details; disabling current path may strand request. [network details][sc17] [discovery][auto-create] | Shortcut; location for protected details if required | 3 | Low; setter needs probe | No | — | 3 |
| I06 Bluetooth | “Turn off Bluetooth on the phone.” | W radio setting, not arbitrary accessory CRUD | Set Bluetooth candidate; no equivalent ordinary-app public radio-toggle API established. [discovery][auto-create] [BLE scope][ble] | Phone Shortcut | 3 | Low, action unverified here | No | — | 2 |
| I07 Cellular data | “Turn mobile data off.” | W cellular-data setting | Set Cellular Data candidate; action/hardware semantics unverified; may break reply path. [discovery][auto-create] | Phone Shortcut; cellular device | 3 | Low | No | — | 2 |
| I08 Cellular plan/default line/roaming | “Use my travel SIM”; “Disable roaming.” | R plans; W supported line/roaming settings | Documented phone cellular-plan actions; carrier/dual-SIM support varies. [actions][sc17] | Phone Shortcut; installed plans | 3 | Low route | No | — | 2 |
| I09 Airplane Mode | “Put my phone in airplane mode.” | W mode | Set Airplane Mode candidate; cannot promise acknowledgment or later remote reversal. [setting trigger][setting-trigger] [discovery][auto-create] | Phone Shortcut | 3 | Low, action probe needed | No | — | 2 |
| I10 Hotspot | “Enable my phone's hotspot.” | W enabled state | Set Personal Hotspot documented; carrier entitlement required. [action][sc16] | Phone Shortcut; carrier support | 3 | Low route | No | — | 2 |
| I11 Hotspot password | “Change my hotspot password.” | R/W supported hotspot password | Documented phone actions; not general Wi-Fi/Passwords vault access. [actions][sc17] | Phone Shortcut; credential access grant | 3 | Low route | No | — | 1 |
| I12 Brightness | “Set my phone brightness to 20%.” | R brightness; W candidate setter | Device Details read documented; Set Brightness candidate requires device probe. [read fields][sc16] | Phone Shortcut | 3 | Low | No | — | 3 |
| I13 Media/ringer volume | “Turn media down”; “Raise the ringer.” | R current exposed volume; W volume channel | Set Volume supports channel choices; distinguish from Silent mode. [channels][sc17] | Phone Shortcut | 3 | Low route | No | — | 3 |
| I14 Flashlight | “Turn on my phone's flashlight.” | W torch state/intensity where supported | Set Torch/Flashlight candidate on phone; exact locked action behavior unverified. [action family][sc16] | Phone Shortcut; torch hardware | 3 | Low | No | — | 2 |
| I15 Appearance | “Use dark mode on my phone.” | W light/dark appearance | Set Appearance candidate; no Mac-to-phone appearance sync assumption. [discovery][auto-create] | Phone Shortcut | 3 | Low, action unverified here | No | — | 2 |
| I16 Night Shift/True Tone | “Turn on Night Shift.” | W supported display modes | Documented Shortcuts actions, device dependent. [actions][sc16] | Phone Shortcut | 3 | Low route | No | — | 2 |
| I17 Always-On Display | “Disable Always-On Display.” | W display setting | Documented action on supported iPhones. [action][sc16] | Phone Shortcut; eligible hardware | 3 | Low route | No | — | 2 |
| I18 Wallpaper selection | “Switch to my travel wallpaper.” | R configured wallpapers; W selection/photo | Documented wallpaper actions; photo permission for new image. [actions][sc16] | Shortcut; Photos if used | 3 | Low route | No | — | 2 |
| I19 Rotation lock | “Lock screen orientation.” | W lock setting | Set Orientation Lock candidate; not general remote orientation force. [discovery][auto-create] | Phone Shortcut | 3 | Low, action unverified here | No | — | 1 |
| I20 Lock/restart/shutdown | “Lock my phone.” | W system action | Documented actions; restart/shutdown may require confirmation and terminate transport. [actions][sc16] | Phone Shortcut; system confirmation as required | 3 | Low, unattended outcome unverified | No | — | 2 |
| I21 AirDrop receiving | “Allow AirDrop from contacts.” | W receive preference | Documented Set AirDrop Receiving; not unattended remote file acceptance. [action][sc16] | Phone Shortcut | 3 | Low route | No | — | 1 |
| I22 Silence unknown callers | “Silence unknown callers.” | W call-screening preference | Documented Shortcut action. [action][sc16] | Phone Shortcut | 3 | Low route | No | — | 2 |
| I23 Announce Notifications | “Stop announcing my alerts.” | W announce preference | Documented action; not access to all notification contents. [action][sc16] | Phone Shortcut; supported output hardware | 3 | Low route | No | — | 1 |
| I24 Open app/URL | “Open my boarding pass app.” | W foreground launch | Shortcut Open App/Open URL; locked device may require interaction. [execution model][auto-create] | Phone Shortcut; target app installed/unlock | 3 | Low | No | — | 2 |
| I25 Phone clipboard | “Copy this address onto my phone.” | R/W phone pasteboard | Phone Clipboard actions candidate; may prompt; Universal Clipboard is a different, nearby route. [discovery][auto-create] [Continuity][universal] | Clipboard/privacy grant; unlock where required | 3 (or 2 nearby) | Low relay | No | — | 2 |
| I26 Screenshot/camera capture | “Take a photo on my phone.” | R image; W returned media | Phone Take Photo/screenshot candidate; camera capture cannot be assumed to work locked or invisibly. [documented camera action][sc17] | Camera/Photos; foreground/unlock as required | 3 | Low; screenshot action unverified here | No | — | 2 |
| I27 Speak/play sound/vibrate | “Make the phone beep so I can find it.” | W supported audio/haptic output | Speak Text/Play Sound/Vibrate candidates; route needs online awake execution and cannot guarantee Silent/Focus bypass. [speech action][sc16] | Shortcut/audio settings | 3 | Low, end-to-end unverified | No | — | 3 |
| I28 Motion/orientation/activity | “Am I walking?” | R device activity/orientation | Documented Get Activity/Get Orientation; not historical location tracking. [actions][sc17] | Motion & Fitness where required | 3 | Low route | No | — | 1 |
| I29 Accessibility settings | “Enable Reduce Motion”; “Make text larger.” | W exposed individual settings only | Shortcuts action-catalog candidates; exact supported settings **unverified**, no general Settings API. [discovery][auto-create] | Phone Shortcut; user setup | 3 | Low | No | — | 2 |
| I30 Disallowed system administration | “Change my passcode”; “Install any app silently.” | Requested W protected system state | Ordinary Shortcuts/companion privileges do not provide general device administration; managed-device deployment is a separate architecture. [Apple device management][mdm] | Managed-device enrollment/supervision for applicable MDM actions | None under current 1–4 design | Impossible as general ordinary-user API | No | — | 1 |
| I31 Supported app settings | “Change Safari's supported settings.” | R/W action-exposed settings only | Get Settings/Change Settings for listed Apple apps; supported fields must be enumerated on-device. [actions][sc18] | Per-app/Shortcut authorization | 3 | Low relay | No | — | 2 |

## Other additions and baseline parity checks

These rows prevent “everything iMCP offers” from overlooking less visible baseline operations, and cover useful Apple ecosystem features outside the requested app groups.

| ID / capability | User asks | R/W | Mechanism on macOS and evidence | Permission/TCC | Reach | Feasibility | iMCP has it? | Other prior art | Value |
|---|---|---|---|---|---|---|---|---|---|
| X01 Calendar baseline parity | “What's tomorrow?”; “Move my meeting.” | R calendars/events; W create/update/delete | EventKit; existing Bridge scope, recurrence/alarm details need semantic comparison, not just tool-count parity. [source][i-calendar] | Calendar full access | 1 | High | `calendars_list`, `events_fetch`, `events_create`, `events_update`, `events_delete` [source][i-calendar] | [orchard-mcp][orchard] | 3 |
| X02 Calendar proximity alerts | “Remind me when I arrive at the venue.” | R/W event alarm configuration | EventKit structured location/proximity alarms where supported; probe actual phone firing. [iMCP implementation][i-calendar] | Calendar; location for firing | 1 | High API, firing unverified | Yes, `events_create` [source][i-calendar] | — | 2 |
| X03 Reminders item baseline | “Remind me Friday”; “Mark this done.” | R/W reminders | EventKit; existing Bridge implementation. [source][i-reminders] | Reminders full access | 1 | High | `reminders_fetch`, `reminders_create`, `reminders_update`, `reminders_delete` [source][i-reminders] | [orchard-mcp][orchard] | 3 |
| X04 Reminders list management | “Rename this list”; “Delete my empty list.” | R/W lists | EventKit EKCalendar save/remove; adds rename/delete beyond baseline create. [source][i-reminders] | Reminders full access; writable account | 1 | High | `reminders_lists`, `reminders_lists_create`, `reminders_lists_rename`, `reminders_lists_delete` [source][i-reminders] | [macos-apps-mcp][apps-art] | 3 |
| X05 Reminders sections/groups | “Group my grocery list into sections.” | W supported organization | OS 27 actions; unavailable through baseline EventKit model as implemented; per-action semantics unverified. [release notes][sc27] | Reminders/Shortcut approval | 1 / 3 | Medium candidate | No | — | 2 |
| X06 Contacts baseline/me | “What's my own contact card?” | R me/search/list; W create/update | CNContactStore and existing source; verify explicit me endpoint parity. [source][i-contacts] | Contacts access | 1 | High | `contacts_me`, `contacts_search`, `contacts_list`, `contacts_create`, `contacts_update` [source][i-contacts] | [orchard-mcp][orchard] | 3 |
| X07 Contact groups | “Create a family group.” | R/W groups and memberships | Contacts CNSaveRequest group/member operations for writable containers. [Contacts][contacts] | Contacts access | 1 if account supports groups | High | No dedicated group writes | [orchard-mcp][orchard] | 2 |
| X08 Contact note field | “Read the note on this contact.” | R/W note only with entitlement | `CNContactNoteKey` restricted entitlement; FDA does not substitute; baseline intentionally excludes it. [Apple entitlement][contact-note] | Contacts + approved note entitlement | 1 | High API, entitlement gate | No verified entitlement-backed note parity | — | 2 |
| X09 Messages baseline read parity | “Read this thread and attachment.” | R local messages/media | Existing `chat.db`/attachment readers; richer mutations already in Bridge scope. [source][i-messages] | FDA | 1 / 2 as configured | Medium reads | `messages_fetch`, `messages_attachment_read` [source][i-messages] | [ReagentX][reagent], [imsg][imsg-art], [BlueBubbles][bluebubbles], [Beeper][beeper] | 3 |
| X10 Pages/Numbers/Keynote | “Create a budget sheet”; “Export these slides.” | R/W documents and export formats | App AppleScript dictionaries; supported object model differs per app. [implementation][orchard] [ScriptingBridge][sb] | AE per app; file access | M; 1 iCloud documents | Medium | No | [orchard-mcp][orchard] | 3 |
| X11 PDF OCR/search/annotations | “Search these scans”; “Highlight this passage.” | R text/pages; W annotations/derived PDF | PDFKit + Vision; non-destructive output file. [PDFKit][pdf] [Vision][vision] | File access | M; 1 Drive file | High | No | [orchard-mcp][orchard] | 3 |
| X12 Spotlight universal file search | “Find everything related to this project.” | R indexed authorized metadata/content | NSMetadataQuery/mdfind; not automatic access to every app's private index. [query][metadata] | File privacy grants | M; downloaded 1 content | High, index coverage conditional | No | [orchard-mcp][orchard] | 3 |
| X13 Time Machine | “Start a backup”; “When was my last backup?” | R supported status; W start/stop | `tmutil` or documented Shortcut; privileges vary; don't promise access to encrypted destinations. [Apple action][sc17] [tmutil man mirror][tmutil] | FDA/admin for applicable operations | M | High documented CLI/action | No | — | 2 |
| X14 Printer/PDF output | “Print this PDF.” | R printers/jobs; W print request | AppKit printing/CUPS tools; print dialog or printer authorization may be needed. [NSPrintOperation][print] | Local Network/file access as applicable | M → printer | High | No | — | 2 |
| X15 Text to speech | “Read this document aloud.” | R text; W audio output | AVSpeechSynthesizer; voice availability varies. [API][tts] | Usually none; file access for source | M / 3 phone action | High local | No | [Hammerspoon][hs] | 2 |
| X16 Music recognition | “What song is that?” | R audio; W recognition result | ShazamKit audio matching, or configured Shazam Shortcut. [framework][shazam] | Microphone if live; service setup as required | M / 3 / 4 | High API conditional | No | — | 2 |
| X17 HomePod intercom | “Tell the house dinner is ready.” | W announcement | Phone Intercom action; not a native Mac HomeKit arbitrary-audio API. [action][sc16] | Home membership/Shortcut permission | 3 → H | Low route | No | — | 2 |
| X18 Apple TV remote | “Pause the Apple TV.” | R/W supported playback/remote operations | Phone Apple TV Remote Shortcut candidate; exact actions/pairing unverified. Private network remote protocols out of policy. [action discovery][auto-create] | Phone/TV pairing; Local Network | 3 | Low, unverified | No | — | 2 |
| X19 Stocks/watchlists | “Add this ticker to my watchlist.” | W supported watchlist entries | OS 26 Shortcuts actions; not a general market-data API. [release notes][sc26] | Shortcut/Stocks configuration | M; 1/3 sync behavior unverified | Medium candidate | No | — | 1 |
| X20 Sports schedule | “When is my team's next match?” | R supported upcoming events | OS 26 Sports action; verify platform/data coverage before exposing. [release notes][sc26] | Shortcut/network | M / 3 | Medium candidate | No | — | 1 |
| X21 Device notifications from companion | “Send a bridge alert to my phone.” | W own notification/push | APNs + UserNotifications; background pushes are throttled and cannot be a guaranteed immediate command bus. [framework][notifications] [limits][push] | APNs entitlement; user notification grant | 4 | High notification API; delivery conditional | No | — | 3 |

## Top 25 by value × feasibility

Scoring: value 1–3 × **high 3 / medium 2 / low 1 / impossible 0**. For mixed rows use the implemented route's weaker grade. Below are **new work packages**, not baseline reads/writes already present. Ties favor breadth of routine use and fewer deployment gates. Scores are planning judgments, not implementation estimates. Companion work retains its documented API score but is explicitly marked; it must not be mistaken for a current Mac capability. Public actions still need real-level feature proofs under SPEC §9.

| Rank | Work package / rows | Value × feasibility | Why prioritize / prerequisite |
|---|---|---|---|
| 1 | Shortcuts inventory + typed runner (S01–S02) | 3 × 3 = 9 | Broad public extension point; enforce action-specific outcomes. |
| 2 | File search + iCloud Drive CRUD (P07–P09) | 3 × 3 = 9 | Everyday documents; coordinate downloads and writes. |
| 3 | Photos search/original export (M01–M02) | 3 × 3 = 9 | Large new personal-data source with public APIs. |
| 4 | Photos import/albums (M03–M04) | 3 × 3 = 9 | Useful writes propagate through Photos sync. |
| 5 | OCR + PDF tools (A07, X11) | 3 × 3 = 9 | Makes files, scans and screenshots useful to clients. |
| 6 | Mac screenshots (D12) | 3 × 3 = 9 | Direct iMCP parity; explicit screen permission. |
| 7 | Mac location + geocoding (L01–L02) | 3 × 3 = 9 | Context for travel/weather; never confuse with phone location. |
| 8 | Maps search/routes/ETA (L03–L04) | 3 × 3 = 9 | Useful across all clients; no private store required. |
| 9 | Weather current/forecast (L08–L09) | 3 × 3 = 9 | iMCP parity; entitlement/attribution setup first. |
| 10 | Mac Clock alarms/timer via Shortcuts (K01, K03) | 3 × 3 = 9 | Public supported action route, despite missing native Clock API. |
| 11 | Mac volume and clipboard (D01, D11) | 3 × 3 = 9 | Straightforward everyday controls. |
| 12 | App launch and network diagnostics (D07–D08) | 3 × 3 = 9 | Useful remote Mac operation with public APIs. |
| 13 | Mac battery/keep awake (D19–D20) | 3 × 3 = 9 | Helps operate the bridge host itself. |
| 14 | Own notifications (D17, X21) | 3 × 3 = 9 | Mac first; phone delivery requires companion/APNs. |
| 15 | Reminders list rename/delete (X04) | 3 × 3 = 9 | Small existing-source extension closes explicit iMCP gap. |
| 16 | Audio transcription (M21) | 3 × 3 = 9 | Extends voicemail/memos to arbitrary authorized recordings. |
| 17 | Local Foundation Models (A03) | 3 × 3 = 9 | Optional local processing; gate on model availability. |
| 18 | Health samples/aggregates (E01–E06) | 3 × 3 = 9, **future companion** | Major phone-specific value; no Mac Health shortcut around consent. |
| 19 | AlarmKit owned alarms (K07–K08) | 3 × 3 = 9, **future companion** | Strong alarm API; command delivery and Clock separation explicit. |
| 20 | Mail read/search/send/organize (C01–C06, P01–P02) | 3 × 2 = 6 | Broad daily value; index schemas and AppleScript are the dependencies. |
| 21 | Call history + voicemail (C07, C10–C12) | 3 × 2 = 6 | Distinct iMCP parity source; downloaded/synced coverage only. |
| 22 | Voice Memos reads/audio/transcripts (M17–M19) | 3 × 2 = 6 | Clear source implementation and immediately useful content. |
| 23 | Safari tabs/history/bookmarks (P11–P14, P17) | 3 × 2 = 6 | High-context browsing data; profile/version handling needed. |
| 24 | Music playback/library (M08–M11) | 3 × 2 = 6 | iMCP parity plus playlists; AE and cloud semantics vary. |
| 25 | Home lights/state/scenes (H01, H03–H06) | 3 × 2 = 6, **deployment-adjusted** | Public API merits 9, but Catalyst signing/distribution gate lowers practical rank. |

Before expanding the phone action catalog, perform a small **route-3 feasibility spike**: self-message → battery query → reply, followed by Focus and alarm creation on the intended device, both locked and unlocked. This is a high-leverage dependency investigation, not an already high-feasibility capability. F07's Screen Time action is another worthwhile early probe. No such actions were run for this research.

## Blocked by Apple

- **Mac Health store:** no iPhone HealthKit access from native Mac or Catalyst merely because the framework links. Use authorized phone-side extraction or an explicit export. [Apple architecture][health-about]
- **Native Mac Clock API / cross-app alarms:** no native Clock framework API identified; **Shortcuts is a documented Mac workaround**. AlarmKit handles the caller's alarms, not Apple's Clock inventory. [Mac actions][sc16] [AlarmManager][alarm-manager]
- **Unrestricted Wallet or Passwords dump:** issuer entitlements, keychain access controls and user authentication still apply with FDA. PassKit, FinanceKit and CSV export are limited exceptions, not full-vault APIs. [Wallet][wallet-config] [Keychain][keychain] [FinanceKit][finance] [export][password-export]
- **General Find My API:** no public account-wide people/device/AirTag API established; private web protocols and private helpers are out of policy. UI is a low-feasibility possibility, not proof of impossibility for every user action. [Apple UI][findmy] [private implementation][pyicloud]
- **Native HomeKit and unrestricted Screen Time settings:** HomeKit needs a different target/entitlements; FamilyControls does not grant CRUD over Apple's existing limits/downtime or unconstrained history export. [HomeKit][home] [FamilyControls][family] [DeviceActivity][activity]
- **System-wide private data/control:** CallKit is not cellular-call-history access; UserNotifications is not all-app notification history; App Intents is not arbitrary Siri RPC; normal phone apps cannot self-grant TCC, change every setting or silently administer the device. [CallKit][callkit] [notifications][notifications] [App Intents][intents] [management boundary][mdm]
- **Guaranteed unattended remote execution:** Message automations, user-interface actions, iCloud sync and background APNs are not a documented always-on remote execution service. Locked-device gates, prompts, power/network availability and throttling remain. [automation][auto] [APNs][push]

## Source notes and verification boundaries

Primary documentation and repository source were fetched during this review. The host is Linux, so macOS application dictionaries, SDK execution, TCC dialogs, physical devices and on-host `man` output could not be tested. Apple-authored man pages are linked where CLI claims matter (the `tmutil` page is an Xcode-man-page mirror). For production, extract each installed app's dictionary with `sdef`, probe framework availability, and gate capabilities on real evidence rather than promoting this catalog to `features.yaml` status. Database access means read-only, WAL-aware, schema-probed access; no row proposes writing Apple-owned SQLite. A source link is evidence of that implementation/documented interface, not independent verification of its behavior on the owner's build.

Open-source references are prior art, not dependency recommendations: licenses and private-API use need separate review before copying code. BlueBubbles' injected helper is excluded by the stated policy; Beeper's AX approach is a different mechanism. `blueutil` explicitly documents private APIs; `brightness` documents undocumented modern display APIs. OpenHaystack is research on Find My-network trackers, not a supported drop-in API for the owner's existing AirTags. [BlueBubbles][bluebubbles] [Beeper][beeper] [blueutil][blueutil] [brightness][brightness] [OpenHaystack][haystack]

The action catalog is necessarily extensible: any installed app can add App Intents. The rows cover the requested Apple domains and the complete reviewed iMCP registration set; they do not claim to enumerate every third-party action or undocumented Apple subsystem. Apple Support pages may update in place. OS 27-only candidates must stay gated away from the repo's documented macOS 26 proofs.

[activity]: https://developer.apple.com/documentation/deviceactivity
[ai-shortcuts]: https://support.apple.com/en-gb/guide/iphone/iph78c41eaf8/ios
[airplay]: https://developer.apple.com/documentation/avkit/avroutepickerview
[alarm]: https://developer.apple.com/documentation/alarmkit
[alarm-manager]: https://developer.apple.com/documentation/alarmkit/alarmmanager
[alarm-sample]: https://developer.apple.com/documentation/alarmkit/scheduling-an-alarm-with-alarmkit
[apps-art]: https://github.com/derekrussell/macos-apps-mcp
[audio]: https://developer.apple.com/documentation/coreaudio
[authservices]: https://developer.apple.com/documentation/authenticationservices
[auto]: https://support.apple.com/guide/shortcuts/enable-or-disable-a-personal-automation-apd602971e63/ios
[auto-create]: https://support.apple.com/en-ie/guide/shortcuts/apdfbdbd7123/ios
[auto-scope]: https://support.apple.com/en-euro/guide/shortcuts/apd690170742/ios
[av]: https://developer.apple.com/documentation/avfoundation
[ax]: https://developer.apple.com/documentation/applicationservices/axuielement
[ax-trust]: https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions
[beeper]: https://github.com/beeper/platform-imessage
[ble]: https://developer.apple.com/documentation/corebluetooth
[bluebubbles]: https://github.com/BlueBubblesApp/bluebubbles-server
[bluetooth]: https://developer.apple.com/documentation/iobluetooth
[blueutil]: https://github.com/toy/blueutil
[brightness]: https://github.com/nriley/brightness
[caffeinate]: https://github.com/apple-oss-distributions/PowerManagement/blob/main/caffeinate/caffeinate.8
[call-record]: https://support.apple.com/en-us/121583
[callkit]: https://developer.apple.com/documentation/callkit
[calls]: https://support.apple.com/guide/phoneapp/make-or-receive-calls-phn28c9d643a/mac
[capture]: https://developer.apple.com/documentation/screencapturekit
[clipboard]: https://developer.apple.com/documentation/appkit/nspasteboard
[contact-note]: https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.contacts.notes
[contacts]: https://developer.apple.com/documentation/contacts
[coord]: https://developer.apple.com/documentation/foundation/nsfilecoordinator
[download]: https://developer.apple.com/documentation/foundation/filemanager/startdownloadingubiquitousitem(at:)
[facetime]: https://developer.apple.com/library/archive/featuredarticles/iPhoneURLScheme_Reference/FacetimeLinks/FacetimeLinks.html
[family]: https://developer.apple.com/documentation/familycontrols
[files]: https://developer.apple.com/documentation/foundation/filemanager
[finance]: https://developer.apple.com/documentation/financekit
[findmy]: https://support.apple.com/guide/findmy-mac/welcome/mac
[focus-api]: https://developer.apple.com/documentation/intents/infocusstatuscenter
[focus-settings]: https://support.apple.com/guide/mac-help/mchlff5da36d/mac
[focus-sync]: https://support.apple.com/en-gb/guide/mac-help/-mchl999b7c1a/mac
[freeform]: https://support.apple.com/guide/freeform/welcome/mac
[freeform-export]: https://support.apple.com/guide/freeform/frfm201900c07/mac
[haystack]: https://github.com/seemoo-lab/openhaystack
[health]: https://developer.apple.com/documentation/healthkit
[health-about]: https://developer.apple.com/documentation/healthkit/about-the-healthkit-framework
[health-auth]: https://developer.apple.com/documentation/healthkit/authorizing-access-to-health-data
[health-bg]: https://developer.apple.com/documentation/healthkit/hkhealthstore/enablebackgrounddelivery(for:frequency:withcompletion:)
[health-export]: https://support.apple.com/en-euro/guide/iphone/iph5ede58c3d/ios
[home]: https://developer.apple.com/documentation/homekit
[hs]: https://github.com/Hammerspoon/hammerspoon
[hs-audio]: https://github.com/Hammerspoon/hammerspoon/tree/master/extensions/audiodevice
[hs-battery]: https://github.com/Hammerspoon/hammerspoon/tree/master/extensions/battery
[hs-wifi]: https://github.com/Hammerspoon/hammerspoon/tree/master/extensions/wifi
[i-calendar]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Calendar.swift
[i-capture]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Capture.swift
[i-contacts]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Contacts.swift
[i-home]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/Shared/Home/HomeTools.swift
[i-location]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Location.swift
[i-mail]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Mail.swift
[i-maps]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Maps.swift
[i-messages]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Messages.swift
[i-music]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Music.swift
[i-notes]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Notes.swift
[i-phone]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Phone.swift
[i-reminders]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Reminders.swift
[i-shortcuts]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Shortcuts.swift
[i-utils]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Utilities.swift
[i-vm]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Voicemail.swift
[i-voice]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/VoiceMemos.swift
[i-weather]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/App/Services/Weather.swift
[imcp]: https://github.com/loopwork/iMCP/blob/1f3798de359cc9e5a83b551c4390b017fb33979d/README.md
[imsg-art]: https://github.com/openclaw/imsg
[intents]: https://developer.apple.com/documentation/appintents
[journal]: https://support.apple.com/en-gb/guide/journal/dev883fc2329/mac
[journaling]: https://developer.apple.com/documentation/journalingsuggestions
[keychain]: https://developer.apple.com/documentation/security/keychain-services
[location]: https://developer.apple.com/documentation/corelocation
[lowpower]: https://developer.apple.com/documentation/foundation/processinfo/islowpowermodeenabled
[mail-art]: https://github.com/isityael/apple-mail-mcp
[managed]: https://developer.apple.com/documentation/managedsettings
[maps]: https://developer.apple.com/documentation/mapkit
[mdm]: https://support.apple.com/guide/deployment/intro-to-device-management-depc0aadd3fe/web
[message-trigger]: https://support.apple.com/en-ae/guide/shortcuts/apdd711f9dff/ios
[metadata]: https://developer.apple.com/documentation/foundation/nsmetadataquery
[models]: https://developer.apple.com/documentation/foundationmodels
[music]: https://developer.apple.com/documentation/musickit
[network]: https://developer.apple.com/documentation/network
[notes-art]: https://github.com/threeplanetssoftware/apple_cloud_notes_parser
[notifications]: https://developer.apple.com/documentation/usernotifications
[orchard]: https://github.com/l22-io/orchard-mcp
[osa-art]: https://github.com/frouaix/MCPAppleScript
[osa-discover]: https://github.com/MayCXC/osa-mcp
[osxphotos]: https://github.com/RhetTbull/osxphotos
[passes]: https://developer.apple.com/documentation/passkit/pkpasslibrary
[passkit]: https://developer.apple.com/documentation/passkit
[password-export]: https://support.apple.com/en-gb/guide/passwords/mchl35b12625/mac
[pdf]: https://developer.apple.com/documentation/pdfkit
[photo]: https://developer.apple.com/documentation/photokit
[pmset]: https://github.com/apple-oss-distributions/PowerManagement/blob/main/pmset/pmset.1
[print]: https://developer.apple.com/documentation/appkit/nsprintoperation
[push]: https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app
[pyicloud]: https://github.com/picklepete/pyicloud
[reading]: https://developer.apple.com/documentation/safariservices/ssreadinglist
[reagent]: https://github.com/ReagentX/imessage-exporter
[rings]: https://developer.apple.com/documentation/healthkit/hkactivitysummaryquery
[safari-art]: https://github.com/MrGo2/icloud-mcp
[safari-db]: https://github.com/ydkhatri/mac_apt/blob/master/plugins/safari.py
[safari-export]: https://support.apple.com/guide/safari/export-browsing-data-ibrwebf10132/mac
[safari-page]: https://developer.apple.com/documentation/safariservices/sfsafaripage
[sb]: https://developer.apple.com/documentation/scriptingbridge
[sc-cli]: https://support.apple.com/en-gu/guide/shortcuts-mac/-apd455c82f02/mac
[sc-format]: https://github.com/sebj/iOS-Shortcuts-Reference
[sc16]: https://support.apple.com/en-us/101583
[sc17]: https://support.apple.com/en-us/111098
[sc26]: https://support.apple.com/en-us/125148
[sc27]: https://support.apple.com/en-us/149045
[setting-trigger]: https://support.apple.com/en-ph/guide/shortcuts/apde31e9638b/ios
[shazam]: https://developer.apple.com/documentation/shazamkit
[speech]: https://developer.apple.com/documentation/speech
[spezi]: https://github.com/StanfordSpezi/SpeziHealthKit
[tags]: https://developer.apple.com/documentation/foundation/urlresourcekey/tagnameskey
[tmutil]: https://keith.github.io/xcode-man-pages/tmutil.8.html
[tsrp]: https://github.com/mattt/TSRP
[tts]: https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer
[universal]: https://support.apple.com/en-us/102430
[vision]: https://developer.apple.com/documentation/vision
[wallet-config]: https://developer.apple.com/documentation/xcode/configuring-wallet-support
[wallet-guide]: https://developer.apple.com/library/archive/documentation/UserExperience/Conceptual/PassKit_PG/Apps.html
[wallpaper]: https://developer.apple.com/documentation/appkit/nsworkspace/setdesktopimageurl(_:for:options:)
[weather]: https://developer.apple.com/documentation/weatherkit
[wifi]: https://developer.apple.com/documentation/corewlan
[workspace]: https://developer.apple.com/documentation/appkit/nsworkspace
[sc18]: https://support.apple.com/en-sg/121131
