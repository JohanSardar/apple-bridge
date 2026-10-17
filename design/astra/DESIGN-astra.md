**Apple Bridge — independent basic design by Astra**

Research date: 2026-10-16. Target: macOS 26 and iOS 26; expanded Screen Time export requires iOS 26.4+.

Build **one reusable Apple-data system with a gateway and optional local replicas**. Ubuntu runs the assistant and a replica; it has no special coordinating role. Another agent can connect directly or install the same replica package on another computer. Promise complete replication of the supported, authorized scope—not a byte-for-byte Apple database or universal access to every Apple feature.

**Components and placement**

- **MacBook Air: Apple gateway.** A Swift app runs in Johan’s logged-in session, with Apple permissions and iCloud/Messages configured. It reads Apple stores and executes writes. A separate, portable gateway service maintains a normalized database, durable change journal, command queue, and API/MCP endpoint. Keep the Mac awake; launch both at login.
- **iPhone: companion app.** A signed Swift app exports HealthKit data and, where available, Screen Time data to the gateway. It retains unsent changes locally. It executes supported Health writes when iOS permits it to run. The Mac cannot substitute for the iPhone’s Health store. [Apple HealthKit availability](https://developer.apple.com/documentation/healthkit/setting-up-healthkit)
- **Ubuntu and future computers: replica service.** Each maintains its own SQLite database, attachment directory, local API/MCP endpoint, synchronization connection, and pending writes. The assistant remains a separate process. Installation needs the gateway address and a scoped pairing credential.
- **MacBook Pro:** remains an ordinary Apple client. Data stored only there needs an additional connector; iCloud cannot mirror unsynced local accounts automatically.

**Synchronization**

Apple’s own synchronization brings supported data onto the Air. Our adapters observe local changes, record normalized upserts/deletions durably, and push them to every subscribed replica over authenticated WebSockets. Consumers query their local database; reads never trigger Apple requests.

Pairing transfers a consistent snapshot plus its journal position, then replays subsequent changes. Persist acknowledgments, stable source identifiers, versions, deletion tombstones, and blob hashes. Reconnect resumes from the last committed position; an expired cursor requires another snapshot. Transfer selected attachments proactively, with explicit download/coverage status.

Push describes the consumer contract. Apple adapters still perform queries: EventKit notifications trigger refetching; Contacts supports change-history tokens; Messages/Notes may need filesystem observation and bounded local polling. Reconcile periodically and after restart to recover missed changes. Permission loss must not masquerade as deletion. [EventKit notifications](https://developer.apple.com/documentation/eventkit/updating-with-notifications), [Contacts history](https://developer.apple.com/documentation/technotes/tn3149-fetching-change-history-events)

Expose source freshness and coverage separately from transport connectivity. Fast gateway-to-replica delivery cannot eliminate iCloud delay, unavailable attachments, or iOS background restrictions.

**Writes**

Every client submits a typed command with an idempotency key and, for edits, an expected record version. The gateway queues and serializes commands to the relevant Apple adapter. Apple stores remain authoritative; adapters reread the resulting state and publish it through the same journal.

Return a command ID immediately, then push queued/applied/failed/unknown outcomes. Reject stale edits as conflicts. Keep pending local edits separate from confirmed data. AppleScript sends lack transactional exactly-once guarantees: after an ambiguous timeout, reconcile instead of blindly sending again. Message delivery/read receipts are separate from successful submission. Offline reads continue; queued writes await the required Apple device.

**Actual coverage**

- **Calendar and Reminders:** EventKit supports reading, creating, editing, deleting, and completing reminders after full-access permission. Respect account/calendar writability. Preserve recurrence and time zones, and declare calendar backfill coverage. Advanced app-specific features require individual checks; EventKit access does not mean complete UI parity. [EventKit access](https://developer.apple.com/documentation/eventkit/accessing-the-event-store)
- **Contacts:** use `CNContactStore`/`CNSaveRequest` for authorized contacts and groups, plus change-history tokens for synchronization. Some source accounts are read-only. [Contacts store](https://developer.apple.com/documentation/contacts/cncontactstore)
- **iMessage:** read the Mac’s Messages SQLite database and attachments read-only with Full Disk Access; send basic messages through Messages AppleScript. [BlueBubbles](https://docs.bluebubbles.app/server) demonstrates this mechanism. History is limited to what reached the Mac. Reactions, edits, unsending, and advanced group operations are separate capabilities, often requiring fragile private APIs. Keep SIP enabled initially. Test basic sending on the exact OS build first: BlueBubbles documents a [macOS 26 send failure](https://github.com/BlueBubblesApp/bluebubbles-server/issues/777).
- **Notes:** use Notes AppleScript for ordinary note/folder reads and basic writes, with optional read-only database indexing. This is an automation adapter with incomplete fidelity: locked content needs unlocking; rich attachments, collaboration, and complex formatting are not guaranteed. Reject unsupported edits instead of silently flattening content. Existing [Notes tooling](https://github.com/ailenshen/apple-notes-mcp) illustrates round-trip limitations; [GitMyNotes](https://github.com/mariochampion/gitmynotes) documents locked-note behavior.
- **Health:** use per-type permissions, `HKObserverQuery` background delivery, and persisted `HKAnchoredObjectQuery` anchors for sample additions/deletions. Export steps, heart rate, sleep, workouts, and other authorized types, preserving units and provenance. Writes can add supported samples; deletion/replacement is restricted to samples this app wrote. Read permission denial can look like missing data. Background timing is not guaranteed, and locked-device reads can fail; remote writes may wait for an app wake or foreground opening. [Observer delivery](https://developer.apple.com/documentation/healthkit/executing-observer-queries), [anchors](https://developer.apple.com/documentation/healthkit/hkanchoredobjectquery), [authorization](https://developer.apple.com/documentation/healthkit/authorizing-access-to-health-data), [deletion](https://developer.apple.com/documentation/healthkit/hkhealthstore/delete(_:withcompletion:)-17hzm), [locked reads](https://developer.apple.com/documentation/healthkit/hkerror/errordatabaseinaccessible)

**Screen Time needs two capability levels**

Ordinary Family Controls/DeviceActivity authorization supports on-device reports showing application/category/domain durations, pickups, and notification counts. Reports run inside a sandbox that forbids networking and exporting sensitive content. Threshold callbacks support monitoring selected usage milestones; they are not a complete activity feed. Thus ordinary authorization cannot supply the desired Linux usage database. [Report restrictions](https://developer.apple.com/documentation/deviceactivity/deviceactivityreport), [threshold callbacks](https://developer.apple.com/documentation/deviceactivity/deviceactivitymonitor/eventdidreachthreshold(_:activity:))

**iOS 26.4+ adds an official export path:** `DeviceActivityData.activityData(filteredBy:using:)`, with the App and Website Usage entitlement and `approvedWithDataAccess`. Customer installations require both EU physical location and an EU Apple Account region. Apple permits development/testing in other regions with its provisioning profiles; do not treat that as unrestricted production availability. Only one app per device can hold this authorization. [Export API](https://developer.apple.com/documentation/deviceactivity/deviceactivitydata/activitydata(filteredby:using:)), [authorization conditions](https://developer.apple.com/documentation/familycontrols/authorizationstatus/approvedwithdataaccess)

Where eligible, export time-bucketed app/category/domain duration, app notification/pickup counts, first pickup, and longest activity interval. `FamilyActivityData` additionally identifies installed apps and visited domains. These APIs do not expose screenshot pixels, notification contents, or a full URL/click history. Schedule exports opportunistically on the phone; there is no documented always-on background feed. [Activity fields](https://developer.apple.com/documentation/deviceactivity/deviceactivitydata), [app/domain identities](https://developer.apple.com/documentation/familycontrols/familyactivitydata)

Screen Time history is not writable. Optional controls can shield selected apps/sites through [ManagedSettings](https://developer.apple.com/documentation/managedsettings/shieldsettings); they do not provide arbitrary editing of Apple’s Screen Time configuration.

**“Everything else” means additional adapters**

Photos, including saved screenshots, can use permissioned [PhotoKit reads/change observation](https://developer.apple.com/documentation/photokit/observing-changes-in-the-photo-library) and [change requests](https://developer.apple.com/documentation/photokit/requesting-changes-to-the-photo-library). Evaluate Mail, iCloud Drive, and Home individually afterward. There is no universal Apple-account API: [CloudKit containers](https://developer.apple.com/documentation/cloudkit/ckcontainer) are scoped to entitled apps.

**API and technology**

Expose versioned REST/OpenAPI endpoints for search, records, attachments, commands, command status, capabilities, and synchronization status; use a separate resumable change stream. MCP wraps the same services with tools such as `messages.send`, `reminders.complete`, `calendar.update`, `notes.create`, and `health.query`. Advertise supported operations and unavailable-data reasons. Use [Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) remotely and an optional stdio wrapper locally; MCP is not the replication protocol.

Choose Swift for Apple adapters; TypeScript/Node.js for the shared gateway/replica service and MCP; SQLite/WAL with full-text search and content-addressed attachment files. Use Tailscale, HTTPS, per-client read/write scopes, encrypted disks/backups, and command auditing. Apple credentials stay on Apple devices.

**Build order, risks, and questions**

First prove macOS 26 message sending, EventKit changes, and iPhone Health export. Then build the journal and two interchangeable replicas; verify disconnect/replay and conflicting writes before adding Notes and eligible Screen Time export. Main risks are Apple API/schema changes, iOS scheduling, Mac sleep/reboots requiring login, and expanded exposure of replicated private data.

Ask Johan:

- What exact iOS version, physical region, and Apple Account region apply?
- Will he maintain a signed iPhone companion app and developer provisioning?
- Which advanced message/Notes features are essential, and are private APIs acceptable?
- What history, attachments, local-only accounts, and Health types must be included?
- Which agents may send/delete/write Health data, and what offline delay is acceptable?

