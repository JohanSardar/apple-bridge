import Foundation
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

private enum CalendarCaptureInvalidation: Error { case invalidated }

@AgendaActor
public final class CalendarSource: Source {
    public nonisolated let name = "calendar"
    public static let operations = ["calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"]
    public static let coverage: JSON = .object(["operations": CalendarParity.coverage, "fields": .string("partial"), "missing": .array(["travel_time: positive native units unverified", "detached store identities", "calendar attachment derivatives", "start location, proposals, comments and extra store alarms: positive native fixtures pending"].map(JSON.string))])
    private let gate: AgendaGate
    private let baselineURL: URL
    private let commandsEnabled: Bool
    private let profilingEnabled: Bool
    private let lookupURL: URL
    private let offeredURL: URL
    private var offered: [String: JSON] = [:]
    private var lookups: [String: JSON] = [:]
    private var savedLookups: [String: JSON] = [:]
    private var savedBaseline: [String: JSON] = [:]
    private var committed: [String: JSON] = [:]
    private var scanning = false
    private var waitingCommands = 0
    private var forceFull = true
    private var pendingScope: CalendarScanScope?
    private var commandRevision: UInt64 = 0
    private var acceptedCommandRevision: UInt64 = 0
    private var commandNeedsCapture = false
    private var lastFullScanStarted: UInt64 = 0
    private var latestDatabaseHintAt: UInt64 = 0
    private var persistence: Task<Void, Never>?
    private var splitCaptures: UInt64 = 0
    private let storeEnrichment = CalendarStoreEnrichment()
    private var materializations: [CalendarOccurrenceKey: CalendarMaterialization] = [:]
    private var verificationRevision: UInt64 = 0
    private var acceptedVerificationRevision: UInt64 = 0

    private func touch(_ event: EKEvent, future: Bool) {
        materializations = materializations.filter { $0.key.identifier != event.calendarItemIdentifier }
        let bounds = CalendarTime.window(Date())
        let original = event.occurrenceDate ?? event.startDate!
        let interval = future ? bounds : DateInterval(start: min(event.startDate, original).addingTimeInterval(-1), end: max(event.endDate, original.addingTimeInterval(1)))
        let scope = CalendarScanScope(interval: interval, calendars: [event.calendar.calendarIdentifier])
        if pendingScope == nil { pendingScope = scope } else { pendingScope?.include(scope) }
    }

    private func persistAccepted(_ values: [String: JSON]) {
        let previous = persistence
        let baselineURL = self.baselineURL
        persistence = Task { @AgendaActor [weak self] in
            await previous?.value
            do {
                try await Task.detached {
                    try JSONEncoder().encode(values).write(to: baselineURL, options: .atomic)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
                }.value
                guard let self else { return }
                try self.saveOffered(self.offered.filter { values[$0.key] == nil })
            } catch {
                self?.gate.trace.record("calendar_baseline_persistence_failed")
            }
        }
    }

    public nonisolated init(gate: AgendaGate, home: URL, commandsEnabled: Bool, profilingEnabled: Bool = false) throws {
        self.gate = gate; self.commandsEnabled = commandsEnabled; self.profilingEnabled = profilingEnabled
        baselineURL = home.appendingPathComponent("calendar-baseline.json")
        lookupURL = home.appendingPathComponent("calendar-lookups.json")
        offeredURL = home.appendingPathComponent("calendar-offered.json")
        if FileManager.default.fileExists(atPath: offeredURL.path) { offered = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: offeredURL)) }
        if FileManager.default.fileExists(atPath: lookupURL.path) { lookups = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: lookupURL)) }
        if FileManager.default.fileExists(atPath: baselineURL.path) {
            committed = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: baselineURL))
        }
        savedLookups = lookups
        savedBaseline = Self.baselineData(committed)
    }

    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return try storeEnrichment.readBlob(handle, offset: offset, length: length)
    }

    public func open() async -> SourceReadiness {
        let authorized = gate.authorized(.event)
        return SourceReadiness(state: authorized ? "ready" : "permission_missing", coverage: Self.coverage, capabilities: authorized && commandsEnabled ? Self.operations : [], reason: authorized ? "" : "calendar_full_access_required")
    }

    private func key(_ event: EKEvent) -> String {
        CalendarTime.identity(event.calendarItemIdentifier, occurrence: event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate : nil, floatingZone: event.isAllDay ? .current : nil)
    }

    private func seriesIdentifier(_ event: EKEvent) -> String {
        let identifier = event.calendarItemExternalIdentifier ?? event.calendarItemIdentifier
        guard event.isDetached, let suffix = identifier.range(of: "/RID=") else { return identifier }
        return String(identifier[..<suffix.lowerBound])
    }

    private func ref(_ calendar: EKCalendar) -> JSON {
        .object(["$ref": .object(["collection": .string("calendar.calendar"), "native_key": .string(calendar.calendarIdentifier)])])
    }

    private func calendarData(_ calendar: EKCalendar) -> JSON {
        .object(["title": .string(calendar.title), "account": .string(calendar.source.sourceIdentifier), "color": .string(calendar.cgColor.map { String(describing: $0.components ?? []) } ?? ""), "writable": .bool(calendar.allowsContentModifications && !calendar.isSubscribed), "type": .string(calendar.isSubscribed ? "subscribed" : ["local", "caldav", "exchange", "subscribed", "birthday"].indices.contains(Int(calendar.type.rawValue)) ? ["local", "caldav", "exchange", "subscribed", "birthday"][Int(calendar.type.rawValue)] : "unknown")])
    }

    private func data(_ event: EKEvent, includeIdentity: Bool = true) -> JSON {
        let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
        var fields: [String: JSON] = ["calendar_id": ref(event.calendar), "title": .string(event.title ?? ""), "notes": event.notes.map(JSON.string) ?? .null, "location": event.location.map(JSON.string) ?? .null, "url": event.url.map { .string($0.absoluteString) } ?? .null, "start": .string(event.isAllDay ? CalendarTime.floating(event.startDate, zone: zone) : timestamp(event.startDate)), "end": .string(event.isAllDay ? CalendarTime.floating(event.endDate, zone: zone) : timestamp(event.endDate)), "all_day": .bool(event.isAllDay), "time_zone": event.timeZone.map { .string($0.identifier) } ?? .null, "is_exception": .bool(event.isDetached), "availability": .number(Double(event.availability.rawValue)), "status": .number(Double(event.status.rawValue)), "attendees": CalendarEventKitFields.participants(event.attendees), "structured_location": CalendarEventKitFields.structuredLocation(event.structuredLocation), "alarms": .array((event.alarms ?? []).map { .object(["absolute_date": $0.absoluteDate.map { .string(timestamp($0)) } ?? .null, "relative_offset": .number($0.relativeOffset)]) })]
        if includeIdentity && (event.hasRecurrenceRules || event.isDetached) {
            fields["series_id"] = .string(seriesIdentifier(event))
            fields["occurrence_date"] = event.occurrenceDate.map { .string(event.isAllDay ? CalendarTime.floating($0, zone: .current) : timestamp($0)) } ?? .null
        }
        var nativeRules = event.recurrenceRules
        if (nativeRules == nil || nativeRules?.isEmpty == true) && event.isDetached {
            let external = seriesIdentifier(event)
            var candidates = gate.store.calendarItems(withExternalIdentifier: external).compactMap { $0 as? EKEvent }
            if let master = gate.store.calendarItem(withIdentifier: event.calendarItemIdentifier) as? EKEvent { candidates.append(master) }
            let anchor = event.occurrenceDate ?? event.startDate!
            let interval = DateInterval(start: anchor.addingTimeInterval(-180 * 86400), end: anchor.addingTimeInterval(180 * 86400))
            candidates += (try? events(interval)) ?? []
            nativeRules = candidates.first(where: {
                Self.canMaterialize($0) && $0.calendar.calendarIdentifier == event.calendar.calendarIdentifier && $0.calendar.source.sourceIdentifier == event.calendar.source.sourceIdentifier && (seriesIdentifier($0) == external || $0.calendarItemIdentifier == event.calendarItemIdentifier) && $0.hasRecurrenceRules
            })?.recurrenceRules
        }
        fields["recurrence"] = nativeRules.map(encodedRules) ?? .null
        fields["owner_participation_status"] = CalendarEventKitFields.ownerParticipationStatus(event.attendees)
        return .object(fields)
    }

    private func encodedRules(_ rules: [EKRecurrenceRule]) -> JSON {
        .array(rules.map { rule in
            func numbers(_ values: [NSNumber]?) -> JSON { .array((values ?? []).map { .number($0.doubleValue) }) }
            return .object(["frequency": .string(["daily", "weekly", "monthly", "yearly"][Int(rule.frequency.rawValue)]), "interval": .number(Double(rule.interval)), "count": .number(Double(rule.recurrenceEnd?.occurrenceCount ?? 0)), "end": rule.recurrenceEnd?.endDate.map { .string(timestamp($0)) } ?? .null, "first_day_of_week": .number(Double(rule.firstDayOfTheWeek)), "days_of_week": .array((rule.daysOfTheWeek ?? []).map { .object(["day": .number(Double($0.dayOfTheWeek.rawValue)), "week": .number(Double($0.weekNumber))]) }), "days_of_month": numbers(rule.daysOfTheMonth), "days_of_year": numbers(rule.daysOfTheYear), "weeks_of_year": numbers(rule.weeksOfTheYear), "months": numbers(rule.monthsOfTheYear), "positions": numbers(rule.setPositions)])
        })
    }

    private func observation(_ collection: String, _ key: String, _ data: JSON, alias: String = "") -> JSON {
        .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(alias), "data": data, "observed_at": .string(timestamp())])
    }

    private func eventObservation(_ event: EKEvent, native: String) -> JSON {
        let occurrence = event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate.map { "|" + (event.isAllDay ? CalendarTime.floating($0, zone: .current) : timestamp($0)) } ?? "" : ""
        let alias = event.calendarItemExternalIdentifier.map { _ in event.calendar.source.sourceIdentifier + "/" + event.calendar.calendarIdentifier + "/" + seriesIdentifier(event) + occurrence } ?? ""
        return observation("calendar.event", native, data(event), alias: alias)
    }

    private func events(_ interval: DateInterval, calendars: Set<String>? = nil) throws -> [EKEvent] {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let queryStarted = CaptureTrace.now()
        let selected = calendars.map { identifiers in gate.store.calendars(for: .event).filter { identifiers.contains($0.calendarIdentifier) } }
        if let selected, selected.isEmpty { return [] }
        let predicate = gate.store.predicateForEvents(withStart: interval.start, end: interval.end, calendars: selected)
        let events = gate.store.events(matching: predicate)
        gate.trace.record("calendar_eventkit_fetch", fields: ["duration_ns": .integer(CaptureTrace.now() - queryStarted), "records": .number(Double(events.count)), "range_days": .number(interval.duration / 86400)])
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return events
    }

    private nonisolated static func baselineData(_ values: [String: JSON]) -> [String: JSON] {
        values.mapValues { value in .object(value.object.filter { $0.key != "observed_at" }) }
    }

    private func saveOffered(_ values: [String: JSON]) throws {
        guard values != offered else { return }
        try JSONEncoder().encode(values).write(to: offeredURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: offeredURL.path)
        offered = values
    }

    private func offer(_ values: [JSON]) throws {
        var pending = offered
        for value in values where value["kind"].string == "upsert" {
            let identity = value["collection"].string + "/" + value["native_key"].string
            if committed[identity] == nil { pending[identity] = value }
        }
        if !values.isEmpty {
            try JSONEncoder().encode(lookups).write(to: lookupURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: lookupURL.path)
        }
        try saveOffered(pending)
    }

    private func save(_ values: [String: JSON]) throws {
        let started = CaptureTrace.now()
        if lookups != savedLookups {
            try JSONEncoder().encode(lookups).write(to: lookupURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: lookupURL.path)
            savedLookups = lookups
            gate.trace.record("calendar_lookups_saved")
        }
        let baseline = Self.baselineData(values)
        guard baseline != savedBaseline else { return }
        try JSONEncoder().encode(values).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
        savedBaseline = baseline
        gate.trace.record("calendar_baseline_saved", fields: ["started_ns": .integer(started), "finished_ns": .integer(CaptureTrace.now())])
    }

    private func scan(into sink: any ObservationSink, live: Bool, command: UInt64 = 0) async throws {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        while scanning || waitingCommands > 0 { try await Task.sleep(for: .milliseconds(10)) }
        if command > 0 && acceptedCommandRevision >= command { return }
        let scope = forceFull || verificationRevision != acceptedVerificationRevision ? nil : pendingScope
        let revision = commandRevision
        pendingScope = nil
        scanning = true
        defer { scanning = false }
        do {
            do {
                try await scanPass(into: sink, live: live, scope: scope)
            } catch CalendarCaptureInvalidation.invalidated {
                gate.trace.record("calendar_capture_invalidated_retry")
                try await scanPass(into: sink, live: live, scope: scope, rebuildAll: true)
            }
            acceptedCommandRevision = max(acceptedCommandRevision, revision)
        } catch {
            if let scope {
                if pendingScope == nil { pendingScope = scope } else { pendingScope?.include(scope) }
            }
            throw error
        }
    }

    private func scanPass(into sink: any ObservationSink, live: Bool, scope: CalendarScanScope?, rebuildAll: Bool = false) async throws {
        let verification = verificationRevision
        let initiallyFull = forceFull
        let rebuild = rebuildAll || forceFull || verification != acceptedVerificationRevision
        let revision = gate.calendarDebounceAt
        let started = timestamp()
        let scan = UUID().uuidString
        let scanNS = CaptureTrace.now()
        var fetchNS: UInt64 = 0, materializeNS: UInt64 = 0, hashNS: UInt64 = 0, coordinationNS: UInt64 = 0
        var reused: UInt64 = 0, rebuilt: UInt64 = 0
        var identityNS: UInt64 = 0, stampNS: UInt64 = 0, rebuildNS: UInt64 = 0
        var stampProfile: CalendarStampProfile? = profilingEnabled ? CalendarStampProfile() : nil
        defer { if profilingEnabled { gate.trace.record("calendar_scan_timing", fields: ["scan": .string(scan), "started_ns": .integer(scanNS), "fetch_ns": .integer(fetchNS), "materialize_ns": .integer(materializeNS), "identity_ns": .integer(identityNS), "stamp_ns": .integer(stampNS), "stamp_native_access_ns": .integer(stampProfile?.nativeNS ?? 0), "stamp_hash_ns": .integer(stampProfile?.hashNS ?? 0), "stamp_notes_ns": .integer(stampProfile?.notesNS ?? 0), "stamp_attendees_ns": .integer(stampProfile?.attendeesNS ?? 0), "stamp_alarms_ns": .integer(stampProfile?.alarmsNS ?? 0), "rebuild_ns": .integer(rebuildNS), "hash_ns": .integer(hashNS), "coordination_ns": .integer(coordinationNS), "reused": .integer(reused), "rebuilt": .integer(rebuilt), "force_full": .bool(initiallyFull), "scoped": .bool(scope != nil)]) } }
        gate.trace.record("calendar_scan_started", fields: ["scan": .string(scan), "live": .bool(live), "force_full": .bool(forceFull), "notification_ns": .integer(gate.calendarNotificationAt), "debounce_ns": .integer(revision), "scoped": .bool(scope != nil)])
        let now = Date()
        let bounds = CalendarTime.window(now)
        var current: [String: JSON] = scope.map { range in committed.filter { !range.covers($0.value) } } ?? [:]
        var seen: Set<String> = []
        var pending: [JSON] = []
        var nextMaterializations = materializations.filter { current[$0.value.identity] != nil }
        var checkedMasters: Set<String> = []
        var series: [String: CalendarSeriesStamp] = [:]
        let floatingZone = TimeZone.current.identifier
        var uncertainSeries: Set<String> = []
        var reusedEvents: [String: EKEvent] = [:]
        let streamInitial = committed.isEmpty
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        for calendar in gate.store.calendars(for: .event) {
            let value = observation("calendar.calendar", calendar.calendarIdentifier, calendarData(calendar))
            current["calendar.calendar/" + calendar.calendarIdentifier] = value
            if forceFull || value["data"] != committed["calendar.calendar/" + calendar.calendarIdentifier]?["data"] { pending.append(value) }
        }
        let chunks = scope.map { range in range.calendars.isEmpty ? [] : bounds.intersection(with: range.interval).map { [$0] } ?? [] } ?? (forceFull || !live ? CalendarTime.chunks(now) : [bounds])
        for (index, chunk) in chunks.enumerated() {
            await gate.waitForReminderRefetch()
            let refetchStarted = CaptureTrace.now()
            let notificationAt = gate.calendarNotificationAt, debounceAt = gate.calendarDebounceAt
            let fetched = try events(chunk, calendars: scope?.calendars)
            fetchNS += CaptureTrace.now() - refetchStarted
            for event in fetched {
                let coordinationStarted = CaptureTrace.now()
                await Task.yield()
                await gate.waitForReminderRefetch()
                coordinationNS += CaptureTrace.now() - coordinationStarted
                try Task.checkCancellation()
                guard Self.canMaterialize(event) else { throw CalendarCaptureInvalidation.invalidated }
                let materializeStarted = profilingEnabled ? CaptureTrace.now() : 0
                let stampStarted = materializeStarted
                let occurrence = CalendarOccurrenceKey(event)
                let identifier = occurrence.identifier
                if checkedMasters.insert(identifier).inserted {
                    let master = event.hasRecurrenceRules ? gate.store.calendarItem(withIdentifier: identifier) as? EKEvent : nil
                    series[identifier] = CalendarSeriesStamp(event, master: master, floatingZone: floatingZone, profile: &stampProfile)
                }
                let stamp = series[identifier].flatMap { CalendarEventStamp.fingerprint(event, series: $0, now: now, profile: &stampProfile) }
                if stamp == nil { uncertainSeries.insert(identifier) }
                if profilingEnabled { stampNS += CaptureTrace.now() - stampStarted }
                if !rebuild, !uncertainSeries.contains(identifier), let stamp, let cached = materializations[occurrence], cached.stamp == stamp, let previous = committed[cached.identity] {
                    guard seen.insert(cached.identity).inserted else { continue }
                    current[cached.identity] = previous
                    reusedEvents[cached.identity] = event
                    let cacheStarted = profilingEnabled ? CaptureTrace.now() : 0
                    nextMaterializations[occurrence] = cached
                    if profilingEnabled { stampNS += CaptureTrace.now() - cacheStarted }
                    reused += 1
                    if profilingEnabled { materializeNS += CaptureTrace.now() - materializeStarted }
                    continue
                }
                let identityStarted = profilingEnabled ? CaptureTrace.now() : 0
                let native = key(event)
                let identity = "calendar.event/" + native
                if profilingEnabled { identityNS += CaptureTrace.now() - identityStarted }
                if !seen.insert(identity).inserted { continue }
                let cacheStarted = profilingEnabled ? CaptureTrace.now() : 0
                nextMaterializations[occurrence] = stamp.map { CalendarMaterialization(stamp: $0, identity: identity) }
                if profilingEnabled { stampNS += CaptureTrace.now() - cacheStarted }
                rebuilt += 1
                let rebuildStarted = profilingEnabled ? CaptureTrace.now() : 0
                try remember(event)
                let value = eventObservation(event, native: native)
                current[identity] = value
                if profilingEnabled { rebuildNS += CaptureTrace.now() - rebuildStarted; materializeNS += CaptureTrace.now() - materializeStarted }
                let hashStarted = CaptureTrace.now()
                let changed = value["data"] != (committed[identity]?["data"] ?? .null)
                hashNS += CaptureTrace.now() - hashStarted
                if forceFull || changed || value["scoped_alias"] != committed[identity]?["scoped_alias"] { pending.append(value) }
            }
            gate.trace.record("refetch", keys: pending.map { $0["native_key"].string }, fields: ["source": .string(name), "notification_ns": .integer(notificationAt), "debounce_ns": .integer(debounceAt), "started_ns": .integer(refetchStarted), "finished_ns": .integer(CaptureTrace.now()), "live": .bool(live || index == 0)])
            gate.trace.record("calendar_chunk", fields: ["index": .number(Double(index)), "records": .number(Double(current.count)), "changes": .number(Double(pending.count))])
            guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
            if streamInitial && !pending.isEmpty {
                await gate.waitForReminderRefetch()
                try offer(pending)
                try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: "", scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: storeEnrichment.handles, live: live || index == 0)
                committed.merge(current) { _, new in new }
                try save(committed)
                try saveOffered(offered.filter { committed[$0.key] == nil })
            }
            if streamInitial { pending = [] }
            await Task.yield()
        }
        for (identity, event) in reusedEvents where uncertainSeries.contains(event.calendarItemIdentifier) {
            await gate.waitForReminderRefetch()
            guard Self.canMaterialize(event) else { throw CalendarCaptureInvalidation.invalidated }
            let rebuildStarted = profilingEnabled ? CaptureTrace.now() : 0
            try remember(event)
            let value = eventObservation(event, native: key(event))
            current[identity] = value
            if value["data"] != committed[identity]?["data"] || value["scoped_alias"] != committed[identity]?["scoped_alias"] { pending.append(value) }
            if profilingEnabled { let elapsed = CaptureTrace.now() - rebuildStarted; materializeNS += elapsed; rebuildNS += elapsed }
            rebuilt += 1; reused -= 1
        }
        nextMaterializations = nextMaterializations.filter { !uncertainSeries.contains($0.key.identifier) }
        let revalidateStarted = CaptureTrace.now()
        for (identity, previous) in committed.merging(offered, uniquingKeysWith: { accepted, _ in accepted }) where current[identity] == nil {
            if let scope, !scope.covers(previous) { continue }
            if current.values.contains(where: { !$0["scoped_alias"].string.isEmpty && $0["scoped_alias"] == previous["scoped_alias"] }) { continue }
            var absent = previous.object
            absent["observed_at"] = .string(timestamp()); absent["revalidated"] = .bool(true)
            var evicted = false
            if previous["collection"].string == "calendar.event" {
                let native = previous["native_key"].string
                if let outside = try lookup(native) {
                    let value = data(outside)
                    let zone = value["all_day"].bool ? TimeZone.current : (TimeZone(identifier: value["time_zone"].string) ?? .current)
                    guard let start = try? CalendarTime.date(value["start"].string, allDay: value["all_day"].bool, zone: zone), let end = try? CalendarTime.date(value["end"].string, allDay: value["all_day"].bool, zone: zone) else { current[identity] = previous; continue }
                    evicted = start >= bounds.end || end <= bounds.start
                    if !evicted { current[identity] = previous; continue }
                } else {
                    let value = previous["data"]
                    let original = value["occurrence_date"].string.isEmpty ? value["start"].string : value["occurrence_date"].string
                    guard let date = try? CalendarTime.date(original, allDay: value["all_day"].bool, zone: .current) else { current[identity] = previous; continue }
                    evicted = date < bounds.start || date >= bounds.end
                }
            }
            absent["kind"] = .string(evicted ? "evict" : "absent")
            pending.append(.object(absent))
        }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        gate.trace.record("calendar_revalidation", fields: ["scan": .string(scan), "duration_ns": .integer(CaptureTrace.now() - revalidateStarted)])
        current = await storeEnrichment.enrich(current, lookups: lookups, force: rebuild)
        pending.removeAll { $0["collection"].string == "calendar.event" && $0["kind"].string == "upsert" }
        for (identity, observation) in current where observation["collection"].string == "calendar.event" {
            if forceFull || observation["data"] != committed[identity]?["data"] || observation["scoped_alias"] != committed[identity]?["scoped_alias"] { pending.append(observation) }
        }
        let unchanged = pending.filter { value in
            let identity = value["collection"].string + "/" + value["native_key"].string
            return value["kind"].string == "upsert" && value["data"] == committed[identity]?["data"] && value["scoped_alias"] == committed[identity]?["scoped_alias"]
        }
        let unchangedKeys = Set(unchanged.map { $0["collection"].string + "/" + $0["native_key"].string })
        pending.removeAll { unchangedKeys.contains($0["collection"].string + "/" + $0["native_key"].string) }
        let captureBytes = try JSONEncoder().encode(JSON.array(pending)).count
        if captureBytes > 240000 {
            splitCaptures += 1
            gate.trace.record("calendar_capture_split", fields: ["count": .integer(splitCaptures), "records": .number(Double(pending.count)), "bytes": .number(Double(captureBytes))])
        }
        if forceFull && !unchanged.isEmpty {
            gate.trace.record("calendar_replay_started", fields: ["scan": .string(scan), "records": .number(Double(unchanged.count))])
            try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: unchanged, scanID: "", scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: storeEnrichment.handles, live: false)
            gate.trace.record("calendar_replay_finished", fields: ["scan": .string(scan)])
        }
        gate.trace.record("calendar_publish_started", keys: pending.map { $0["native_key"].string }, fields: ["scan": .string(scan), "records": .number(Double(pending.count)), "bytes": .number(Double(captureBytes))])
        try offer(pending)
        try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: scan, scanStartedAt: started, scope: ["calendar.calendar", "calendar.event"], completionEvidence: "complete", blobHandles: storeEnrichment.handles, live: true)
        gate.trace.record("calendar_publish_accepted", fields: ["scan": .string(scan)])
        let persist = forceFull || !pending.isEmpty
        committed = current; forceFull = false
        materializations = nextMaterializations.filter { current[$0.value.identity] != nil }
        if scope == nil { lastFullScanStarted = scanNS; acceptedVerificationRevision = verification }
        if persist { persistAccepted(current) }
        gate.trace.record("calendar_scan_complete", fields: ["records": .number(Double(current.count)), "capture_splits": .integer(splitCaptures), "series_stamps": .number(Double(checkedMasters.count))])
 let readiness=await open()
 try await sink.sourceState(source:name,state:readiness.state,coverage:readiness.coverage,capabilities:readiness.capabilities,reason:readiness.reason)
    }

    public func reconcile(into sink: any ObservationSink) async throws {
        let revision = commandNeedsCapture ? commandRevision : 0
        commandNeedsCapture = false
        try await scan(into: sink, live: true, command: revision)
    }

    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        forceFull = true
        let changes = gate.changes(calendar: true)
        let databaseHints = CalendarDatabaseHints(trace: gate.trace)
        let hints = Task { @AgendaActor [weak self, gate] in
            for await delivered in databaseHints.changes {
                if let self { self.latestDatabaseHintAt = max(self.latestDatabaseHintAt, delivered) }
                gate.calendarHint(delivered)
            }
        }
        defer { hints.cancel(); databaseHints.close() }
        try await scan(into: sink, live: false)
        let periodic = Task { @AgendaActor in
            var day = Calendar.current.startOfDay(for: Date())
            var last = ContinuousClock.now
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                let today = Calendar.current.startOfDay(for: Date())
                let now = ContinuousClock.now
                if today != day || last.duration(to: now) >= .seconds(900) { day = today; last = now; self.verificationRevision += 1; self.gate.reconcile() }
            }
        }
        defer { periodic.cancel() }
        var handledHint: UInt64 = 0
        for await _ in changes {
            try Task.checkCancellation()
            let hint = gate.calendarDebounceAt
            if verificationRevision == acceptedVerificationRevision && hint != handledHint && Self.hintIsCovered(first: gate.calendarNotificationAt, latest: latestDatabaseHintAt, scanStarted: lastFullScanStarted) {
                handledHint = hint
                continue
            }
            handledHint = hint
            try await scan(into: sink, live: true)
        }
    }

    public nonisolated static func canMaterialize(_ event: EKEvent) -> Bool {
        guard event.startDate != nil, event.endDate != nil,
              let calendar = event.calendar, calendar.source != nil else { return false }
        return true
    }

    public nonisolated static func hintIsCovered(first: UInt64, latest: UInt64, scanStarted: UInt64) -> Bool {
        max(first, latest) < scanStarted
    }

    private func remember(_ event: EKEvent) throws {
        guard Self.canMaterialize(event), let calendar = event.calendar, let account = calendar.source else { throw CalendarCaptureInvalidation.invalidated }
        lookups[key(event)] = .object(["event_identifier": .string(event.eventIdentifier ?? ""), "calendar": .string(calendar.calendarIdentifier), "account": .string(account.sourceIdentifier), "external": .string(event.calendarItemExternalIdentifier == nil ? "" : seriesIdentifier(event)), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null])
    }

    private func lookup(_ native: String) throws -> EKEvent? {
        guard gate.authorized(.event), let identity = lookups[native] else { return nil }
        func matches(_ event: EKEvent) throws -> Bool {
            guard Self.canMaterialize(event), let calendar = event.calendar, let account = calendar.source else { throw CalendarCaptureInvalidation.invalidated }
            guard calendar.calendarIdentifier == identity["calendar"].string, account.sourceIdentifier == identity["account"].string else { return false }
            if key(event) == native { return true }
            guard !identity["external"].string.isEmpty, seriesIdentifier(event) == identity["external"].string else { return false }
            if event.isAllDay, let original = event.occurrenceDate, let civil = native.split(separator: "|").last, civil.count == 10 {
                return CalendarTime.floating(original, zone: .current) == String(civil)
            }
            return (event.occurrenceDate.map { JSON.string(timestamp($0)) } ?? .null) == identity["occurrence"]
        }
        if native.contains("|"), let original = parseTimestamp(identity["occurrence"].string) {
            var originals = [original]
            if let civil = native.split(separator: "|").last, civil.count == 10 {
                originals += [TimeZone.current, TimeZone(secondsFromGMT: 0)!].compactMap { try? CalendarTime.date(String(civil), allDay: true, zone: $0) }
            }
            for anchor in Set(originals) {
                let alias = identity["external"].string + "/RID=" + String(Int(anchor.timeIntervalSinceReferenceDate))
                if let event = try gate.store.calendarItems(withExternalIdentifier: alias).compactMap({ $0 as? EKEvent }).first(where: matches) { return event }
            }
            let anchor = originals.count > 1 ? originals[1] : original
            if let event = try events(DateInterval(start: anchor.addingTimeInterval(-1), end: anchor.addingTimeInterval(1))).first(where: matches) { return event }
            if let event = gate.store.event(withIdentifier: identity["event_identifier"].string), event.isDetached, try matches(event) { return event }
            return nil
        }
        if let event = gate.store.event(withIdentifier: identity["event_identifier"].string), try matches(event) { return event }
        if !identity["external"].string.isEmpty {
            if let event = try gate.store.calendarItems(withExternalIdentifier: identity["external"].string).compactMap({ $0 as? EKEvent }).first(where: matches) { return event }
        }
        return nil
    }

    private func knownAbsent(_ native: String) -> Bool {
        guard gate.authorized(.event), let identity = lookups[native], !identity["event_identifier"].string.isEmpty else { return false }
        if native.contains("|") {
            let calendar = identity["calendar"].string, account = identity["account"].string
            guard gate.store.calendar(withIdentifier: calendar)?.source?.sourceIdentifier == account else { return false }
            let original: Date?
            if let civil = native.split(separator: "|").last, civil.count == 10 {
                original = try? CalendarTime.date(String(civil), allDay: true, zone: .current)
            } else { original = parseTimestamp(identity["occurrence"].string) }
            guard let original, let captured = try? events(DateInterval(start: original.addingTimeInterval(-1), end: original.addingTimeInterval(1)), calendars: [calendar]) else { return false }
            guard captured.allSatisfy(Self.canMaterialize) else { return false }
            let missing: Bool
            do { missing = try lookup(native) == nil } catch { return false }
            return gate.authorized(.event) && gate.store.calendar(withIdentifier: calendar)?.source?.sourceIdentifier == account && missing && !captured.contains { key($0) == native }
        }
        return gate.store.event(withIdentifier: identity["event_identifier"].string) == nil
    }

    private func fresh(_ native: String) throws -> EKEvent {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        if let event = try lookup(native) { return event }
        for chunk in CalendarTime.chunks(Date()) {
            let captured = try events(chunk)
            guard captured.allSatisfy(Self.canMaterialize) else { throw SourceFailure.unavailable }
            if let event = captured.first(where: { key($0) == native }) { try remember(event); return event }
        }
        throw SourceFailure.conflict
    }

    private func writable(_ key: String) throws -> EKCalendar {
        guard let calendar = gate.store.calendar(withIdentifier: key), calendar.allowsContentModifications, !calendar.isSubscribed else { throw SourceFailure.unsupported }
        return calendar
    }

    private func args(_ action: JSON) -> JSON { action["calendar_args"] }

    public func prepare(_ command: JSON) async throws -> Prepared {
        guard commandsEnabled else { throw SourceFailure.unsupported }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let action = command["action"]
        guard Self.operations.contains(action["op"].string), args(action)["span"].string != "all" else { throw SourceFailure.unsupported }
        var before = JSON.null
        var existing: EKEvent? = nil
        if !action["native_key"].string.isEmpty {
            let event = try fresh(action["native_key"].string)
            existing = event
            before = data(event)
            _ = try writable(event.calendar.calendarIdentifier)
            guard action["expected_list_key"].string == event.calendar.calendarIdentifier else { throw SourceFailure.conflict }
            var expected = command["expected_state"].object, actual = before.object
            expected.removeValue(forKey: "calendar_id"); actual.removeValue(forKey: "calendar_id")
            for field in CalendarStoreEnrichment.fields { expected.removeValue(forKey: field) }
            guard expected == actual else { throw SourceFailure.conflict }
        }
        if action["op"].string != "calendar.create_calendar" {
            let calendar = try writable(action["list_key"].string)
            if action["op"].string != "calendar.delete_event" {
                _ = try requestedGoal(existing ?? EKEvent(eventStore: gate.store), args(action), calendar: calendar)
            }
        }
        else { guard gate.store.defaultCalendarForNewEvents?.allowsContentModifications == true else { throw SourceFailure.unsupported } }
        return Prepared(nativeState: before, fingerprint: command["request_hash"].string, action: action)
    }

    private func patch(_ event: EKEvent, _ args: JSON) throws {
        if let value = args.object["title"] { event.title = value.string }
        if let value = args.object["notes"] { event.notes = value == .null ? nil : value.string }
        if let value = args.object["location"] { event.location = value == .null ? nil : value.string }
        if let value = args.object["url"] { event.url = value == .null ? nil : URL(string: value.string) }
        if let value = args.object["all_day"] { event.isAllDay = value.bool }
        if let value = args.object["time_zone"] {
            if value == .null || value.string.isEmpty { event.timeZone = nil }
            else {
                guard let zone = TimeZone(identifier: value.string) else { throw SourceFailure.invalid }
                event.timeZone = zone
            }
        }
        let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
        if let value = args.object["start"] { event.startDate = try CalendarTime.date(value.string, allDay: event.isAllDay, zone: zone) }
        if let value = args.object["end"] { event.endDate = try CalendarTime.date(value.string, allDay: event.isAllDay, zone: zone) }
        guard let start = event.startDate, let end = event.endDate, end > start else { throw SourceFailure.invalid }
        if let value = args.object["availability"] {
            guard let availability = EKEventAvailability(rawValue: Int(value.number)) else { throw SourceFailure.invalid }
            event.availability = availability
        }
        if let value = args.object["alarms"] {
            event.alarms = try value.array.map { item in
                if !item["absolute_date"].string.isEmpty { guard let date = parseTimestamp(item["absolute_date"].string) else { throw SourceFailure.invalid }; return EKAlarm(absoluteDate: date) }
                return EKAlarm(relativeOffset: item["relative_offset"].number)
            }
        }
        if let value = args.object["recurrence"] {
            event.recurrenceRules = try value.array.map { item in
                let frequencies: [String: EKRecurrenceFrequency] = ["daily": .daily, "weekly": .weekly, "monthly": .monthly, "yearly": .yearly]
                guard let frequency = frequencies[item["frequency"].string], let interval = Int(exactly: item["interval"].number), interval > 0 else { throw SourceFailure.invalid }
                var end: EKRecurrenceEnd?
                if item["count"].number > 0 {
                    guard let count = Int(exactly: item["count"].number) else { throw SourceFailure.invalid }
                    if args["span"].string == "future" {
                        let zone = event.isAllDay ? TimeZone(secondsFromGMT: 0)! : (event.timeZone ?? .current)
                        let origin = event.isAllDay ? try CalendarTime.date(CalendarTime.floating(start, zone: .current), allDay: true, zone: zone) : start
                        end = EKRecurrenceEnd(end: try CalendarTime.recurrenceEnd(start: origin, frequency: item["frequency"].string, interval: interval, count: count, zone: zone))
                    } else { end = EKRecurrenceEnd(occurrenceCount: count) }
                }
                else if let date = parseTimestamp(item["end"].string) { end = EKRecurrenceEnd(end: date) }
                return EKRecurrenceRule(recurrenceWith: frequency, interval: interval, end: end)
            }
        }
    }

    private func requestedGoal(_ event: EKEvent, _ request: JSON, calendar: EKCalendar, baseline: JSON = .null) throws -> JSON {
        let probe = EKEvent(eventStore: gate.store)
        probe.calendar = calendar
        probe.isAllDay = event.isAllDay
        probe.timeZone = event.timeZone
        probe.startDate = event.startDate
        probe.endDate = event.endDate
        if baseline != .null {
            let values = baseline.object.filter { ["start", "end", "all_day", "time_zone"].contains($0.key) }
            try patch(probe, .object(values))
        }
        try patch(probe, request)
        var result = data(probe, includeIdentity: false).object.filter { request.object[$0.key] != nil && !["id", "span"].contains($0.key) }
        if request.object["recurrence"] != nil { result["recurrence"] = encodedRules(probe.recurrenceRules ?? []) }
        return .object(result)
    }

    private func nativeResult(_ event: EKEvent) -> JSON {
        var result = data(event).object
        result["recurrence"] = encodedRules(event.recurrenceRules ?? [])
        return .object(result)
    }

    private func validateNative(_ prepared: Prepared) throws -> EKEvent? {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let action = prepared.action
        guard Self.operations.contains(action["op"].string) else { throw SourceFailure.unsupported }
        let existing = action["native_key"].string.isEmpty ? nil : try fresh(action["native_key"].string)
        if let event = existing {
            guard data(event) == prepared.nativeState else { throw SourceFailure.conflict }
            _ = try writable(event.calendar.calendarIdentifier)
        }
        if action["op"].string != "calendar.create_calendar" {
            let calendar = try writable(action["list_key"].string)
            if action["op"].string != "calendar.delete_event" {
                _ = try requestedGoal(existing ?? EKEvent(eventStore: gate.store), args(action), calendar: calendar)
            }
        }
        return existing
    }

    public func validate(_ prepared: Prepared) async throws {
        _ = try validateNative(prepared)
    }

    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let waitStarted = CaptureTrace.now()
        waitingCommands += 1
        defer { waitingCommands -= 1 }
        while scanning { try await Task.sleep(for: .milliseconds(10)) }
        gate.trace.record("calendar_command_scan_wait", fields: ["duration_ns": .integer(CaptureTrace.now() - waitStarted), "operation": .string(prepared.action["op"].string)])
        let validated = try validateNative(prepared)
        let action = prepared.action, operation = prepared.action["op"].string
        var native = action["native_key"].string
        var actualResult = JSON.null
        if operation == "calendar.create_calendar" {
            guard let source = gate.store.defaultCalendarForNewEvents?.source else { throw SourceFailure.unavailable }
            let calendar = EKCalendar(for: .event, eventStore: gate.store)
            calendar.source = source; calendar.title = args(action)["title"].string
            gate.trace.record("calendar_native_save_started", fields: ["operation": .string(operation)])
            try gate.store.saveCalendar(calendar, commit: true)
            native = calendar.calendarIdentifier
            pendingScope = pendingScope ?? CalendarScanScope(interval: CalendarTime.window(Date()), calendars: [])
            gate.trace.record("calendar_native_effect", keys: [native], fields: ["operation": .string(operation)])
            guard let observed = gate.store.calendar(withIdentifier: native), observed.title == calendar.title else { throw SourceFailure.unavailable }
            actualResult = calendarData(observed)
        } else {
            let event = operation == "calendar.create_event" ? EKEvent(eventStore: gate.store) : try validated ?? fresh(native)
            if operation != "calendar.create_event" { touch(event, future: args(action)["span"].string == "future") }
            event.calendar = try writable(action["list_key"].string)
            let span: EKSpan = args(action)["span"].string == "future" ? .futureEvents : .thisEvent
            let goal = operation == "calendar.delete_event" ? JSON.null : try requestedGoal(event, args(action), calendar: event.calendar)
            let intendedAllDay = args(action).object["all_day"]?.bool ?? event.isAllDay
            if operation == "calendar.delete_event" {
                gate.trace.record("calendar_native_save_started", fields: ["operation": .string(operation)])
                try gate.store.remove(event, span: span, commit: true)
            } else {
                try patch(event, args(action))
                gate.trace.record("calendar_native_save_started", fields: ["operation": .string(operation)])
                try gate.store.save(event, span: span, commit: true)
                if Self.canMaterialize(event) { native = key(event) }
            }
            if operation != "calendar.delete_event" {
                if Self.canMaterialize(event) { touch(event, future: span == .futureEvents || event.hasRecurrenceRules) }
                else { pendingScope = nil }
            }
            gate.trace.record("calendar_native_effect", keys: [native], fields: ["operation": .string(operation)])
            if operation == "calendar.delete_event" {
                guard (try? fresh(native)) == nil else { throw SourceFailure.unavailable }
            } else {
                commandRevision += 1
                commandNeedsCapture = true
                let observed = try? fresh(native)
                let actual = observed.map(nativeResult) ?? .null
                if let observed { native = key(observed) }
                return CalendarPostcondition.evidence(native: native, fingerprint: prepared.fingerprint, actual: actual, expected: goal, allDay: intendedAllDay)
            }
        }
        commandRevision += 1
        commandNeedsCapture = true
        return .object(["native_key": .string(native), "fingerprint": .string(prepared.fingerprint), "observed": .bool(true), "native_result": actualResult, "absent": .bool(operation == "calendar.delete_event")])
    }

    public func confirmAbsence(_ native: String, into sink: any ObservationSink) async throws {
        guard gate.authorized(.event), (try? fresh(native)) == nil, knownAbsent(native) else { throw SourceFailure.conflict }
        let absent: JSON = .object(["kind": .string("absent"), "collection": .string("calendar.event"), "native_key": .string(native), "observed_at": .string(timestamp()), "revalidated": .bool(true)])
        try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: [absent], scanID: UUID().uuidString, scanStartedAt: timestamp(), scope: ["calendar.event"], completionEvidence: "complete", blobHandles: storeEnrichment.handles, live: true)
    }

    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard gate.authorized(.event) else {return .null}
 let action=evidence["prepared"]["action"]
 let key=evidence["native_key"].string.isEmpty ? action["native_key"].string : evidence["native_key"].string
 guard !key.isEmpty else {return .null}
        if action["op"].string == "calendar.create_calendar" {
            guard gate.store.calendar(withIdentifier: key)?.title == args(action)["title"].string else { return .null }
        } else if action["op"].string == "calendar.delete_event" {
            guard (try? fresh(key)) == nil, knownAbsent(key) else { return .null }
        } else {
            guard let current = try? fresh(key) else { return .null }
            let expected: JSON
            if evidence["native_expected"] != .null { expected = evidence["native_expected"] }
            else {
                guard let goal = try? requestedGoal(current, args(action), calendar: try writable(action["list_key"].string), baseline: evidence["prepared"]["native_state"]) else { return .null }
                expected = goal
            }
            var result = CalendarPostcondition.evidence(native: key, fingerprint: evidence["fingerprint"].string, actual: nativeResult(current), expected: expected, allDay: evidence["native_expected_all_day"] == .null ? current.isAllDay : evidence["native_expected_all_day"].bool).object
            result["prepared"] = evidence["prepared"]
            return .object(result)
        }
var result=evidence.object
 result["native_key"] = .string(key);result["observed"] = .bool(true);result["absent"] = .bool(action["op"].string=="calendar.delete_event")
 return .object(result)
    }
}
