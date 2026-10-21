import Foundation
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@AgendaActor
public final class CalendarSource: Source {
    public nonisolated let name = "calendar"
    public static let operations = ["calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"]
    public static let coverage: JSON = .object(["fields": .string("complete"), "missing": .array([])])
    private let gate: AgendaGate
    private let baselineURL: URL
    private let commandsEnabled: Bool
    private let lookupURL: URL
    private let offeredURL: URL
    private var offered: [String: JSON] = [:]
    private var lookups: [String: JSON] = [:]
    private var savedLookups: [String: JSON] = [:]
    private var savedBaseline: [String: JSON] = [:]
    private var committed: [String: JSON] = [:]
    private var scanning = false
    private var forceFull = true
    private var pendingScope: CalendarScanScope?
    private var commandRevision: UInt64 = 0
    private var acceptedCommandRevision: UInt64 = 0
    private var commandNeedsCapture = false
    private var lastFullScanStarted: UInt64 = 0
    private var persistence: Task<Void, Never>?

    private func touch(_ event: EKEvent, future: Bool) {
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

    public nonisolated init(gate: AgendaGate, home: URL, commandsEnabled: Bool) throws {
        self.gate = gate; self.commandsEnabled = commandsEnabled
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

    private func data(_ event: EKEvent) -> JSON {
        let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
        var fields: [String: JSON] = ["calendar_id": ref(event.calendar), "title": .string(event.title ?? ""), "notes": event.notes.map(JSON.string) ?? .null, "location": event.location.map(JSON.string) ?? .null, "url": event.url.map { .string($0.absoluteString) } ?? .null, "start": .string(event.isAllDay ? CalendarTime.floating(event.startDate, zone: zone) : timestamp(event.startDate)), "end": .string(event.isAllDay ? CalendarTime.floating(event.endDate, zone: zone) : timestamp(event.endDate)), "all_day": .bool(event.isAllDay), "time_zone": event.timeZone.map { .string($0.identifier) } ?? .null, "is_exception": .bool(event.isDetached), "availability": .number(Double(event.availability.rawValue)), "status": .number(Double(event.status.rawValue)), "attendees": .array((event.attendees ?? []).map { .object(["name": $0.name.map(JSON.string) ?? .null, "url": .string($0.url.absoluteString), "status": .number(Double($0.participantStatus.rawValue)), "role": .number(Double($0.participantRole.rawValue)), "type": .number(Double($0.participantType.rawValue))]) }), "alarms": .array((event.alarms ?? []).map { .object(["absolute_date": $0.absoluteDate.map { .string(timestamp($0)) } ?? .null, "relative_offset": .number($0.relativeOffset)]) })]
        if event.hasRecurrenceRules || event.isDetached {
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
                $0.calendar.calendarIdentifier == event.calendar.calendarIdentifier && $0.calendar.source.sourceIdentifier == event.calendar.source.sourceIdentifier && (seriesIdentifier($0) == external || $0.calendarItemIdentifier == event.calendarItemIdentifier) && $0.hasRecurrenceRules
            })?.recurrenceRules
        }
        fields["recurrence"] = nativeRules.map { rules in .array(rules.map { rule in
            func numbers(_ values: [NSNumber]?) -> JSON { .array((values ?? []).map { .number($0.doubleValue) }) }
            return .object(["frequency": .string(["daily", "weekly", "monthly", "yearly"][Int(rule.frequency.rawValue)]), "interval": .number(Double(rule.interval)), "count": .number(Double(rule.recurrenceEnd?.occurrenceCount ?? 0)), "end": rule.recurrenceEnd?.endDate.map { .string(timestamp($0)) } ?? .null, "first_day_of_week": .number(Double(rule.firstDayOfTheWeek)), "days_of_week": .array((rule.daysOfTheWeek ?? []).map { .object(["day": .number(Double($0.dayOfTheWeek.rawValue)), "week": .number(Double($0.weekNumber))]) }), "days_of_month": numbers(rule.daysOfTheMonth), "days_of_year": numbers(rule.daysOfTheYear), "weeks_of_year": numbers(rule.weeksOfTheYear), "months": numbers(rule.monthsOfTheYear), "positions": numbers(rule.setPositions)])
        }) } ?? .null
        return .object(fields)
    }

    private func observation(_ collection: String, _ key: String, _ data: JSON, alias: String = "") -> JSON {
        .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(alias), "data": data, "observed_at": .string(timestamp())])
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
        while scanning { try await Task.sleep(for: .milliseconds(10)) }
        if command > 0 && acceptedCommandRevision >= command { return }
        let scope = forceFull ? nil : pendingScope
        let revision = commandRevision
        pendingScope = nil
        scanning = true
        defer { scanning = false }
        do {
            try await scanPass(into: sink, live: live, scope: scope)
            acceptedCommandRevision = max(acceptedCommandRevision, revision)
        } catch {
            if let scope {
                if pendingScope == nil { pendingScope = scope } else { pendingScope?.include(scope) }
            }
            throw error
        }
    }

    private func scanPass(into sink: any ObservationSink, live: Bool, scope: CalendarScanScope?) async throws {
        let revision = gate.calendarDebounceAt
        let started = timestamp()
        let scan = UUID().uuidString
        let scanNS = CaptureTrace.now()
        var fetchNS: UInt64 = 0, materializeNS: UInt64 = 0, hashNS: UInt64 = 0
        defer { gate.trace.record("calendar_scan_timing", fields: ["scan": .string(scan), "started_ns": .integer(scanNS), "fetch_ns": .integer(fetchNS), "materialize_ns": .integer(materializeNS), "hash_ns": .integer(hashNS), "force_full": .bool(forceFull), "scoped": .bool(scope != nil)]) }
        gate.trace.record("calendar_scan_started", fields: ["scan": .string(scan), "live": .bool(live), "force_full": .bool(forceFull), "notification_ns": .integer(gate.calendarNotificationAt), "debounce_ns": .integer(revision), "scoped": .bool(scope != nil)])
        let now = Date()
        let bounds = CalendarTime.window(now)
        var current: [String: JSON] = scope.map { range in committed.filter { !range.covers($0.value) } } ?? [:]
        var seen: Set<String> = []
        var pending: [JSON] = []
        let streamInitial = committed.isEmpty
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        for calendar in gate.store.calendars(for: .event) {
            let value = observation("calendar.calendar", calendar.calendarIdentifier, calendarData(calendar))
            current["calendar.calendar/" + calendar.calendarIdentifier] = value
            if forceFull || value["data"] != committed["calendar.calendar/" + calendar.calendarIdentifier]?["data"] { pending.append(value) }
        }
        let chunks = scope.map { range in range.calendars.isEmpty ? [] : bounds.intersection(with: range.interval).map { [$0] } ?? [] } ?? CalendarTime.chunks(now)
        for (index, chunk) in chunks.enumerated() {
            await gate.waitForReminderRefetch()
            let refetchStarted = CaptureTrace.now()
            let notificationAt = gate.calendarNotificationAt, debounceAt = gate.calendarDebounceAt
            let fetched = try events(chunk, calendars: scope?.calendars)
            fetchNS += CaptureTrace.now() - refetchStarted
            for event in fetched {
                await Task.yield()
                await gate.waitForReminderRefetch()
                try Task.checkCancellation()
                let materializeStarted = CaptureTrace.now()
                let native = key(event)
                remember(event)
                let identity = "calendar.event/" + native
                if !seen.insert(identity).inserted { continue }
                let occurrence = event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate.map { "|" + (event.isAllDay ? CalendarTime.floating($0, zone: .current) : timestamp($0)) } ?? "" : ""
                let alias = event.calendarItemExternalIdentifier.map { _ in event.calendar.source.sourceIdentifier + "/" + event.calendar.calendarIdentifier + "/" + seriesIdentifier(event) + occurrence } ?? ""
                let value = observation("calendar.event", native, data(event), alias: alias)
                current[identity] = value
                materializeNS += CaptureTrace.now() - materializeStarted
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
                try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: "", scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: [:], live: live || index == 0)
                committed.merge(current) { _, new in new }
                try save(committed)
                try saveOffered(offered.filter { committed[$0.key] == nil })
            }
            if streamInitial { pending = [] }
            await Task.yield()
        }
        let revalidateStarted = CaptureTrace.now()
        for (identity, previous) in committed.merging(offered, uniquingKeysWith: { accepted, _ in accepted }) where current[identity] == nil {
            if let scope, !scope.covers(previous) { continue }
            if current.values.contains(where: { !$0["scoped_alias"].string.isEmpty && $0["scoped_alias"] == previous["scoped_alias"] }) { continue }
            var absent = previous.object
            absent["observed_at"] = .string(timestamp()); absent["revalidated"] = .bool(true)
            var evicted = false
            if previous["collection"].string == "calendar.event" {
                let native = previous["native_key"].string
                if let outside = lookup(native) {
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
        let unchanged = pending.filter { value in
            let identity = value["collection"].string + "/" + value["native_key"].string
            return value["kind"].string == "upsert" && value["data"] == committed[identity]?["data"] && value["scoped_alias"] == committed[identity]?["scoped_alias"]
        }
        let unchangedKeys = Set(unchanged.map { $0["collection"].string + "/" + $0["native_key"].string })
        pending.removeAll { unchangedKeys.contains($0["collection"].string + "/" + $0["native_key"].string) }
        let captureBytes = try JSONEncoder().encode(JSON.array(pending)).count
        if captureBytes > 240000 {
            gate.trace.record("calendar_atomic_capture_too_large", fields: ["records": .number(Double(pending.count))])
            throw SourceFailure.unavailable
        }
        if forceFull && !unchanged.isEmpty {
            gate.trace.record("calendar_replay_started", fields: ["scan": .string(scan), "records": .number(Double(unchanged.count))])
            try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: unchanged, scanID: "", scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: [:], live: false)
            gate.trace.record("calendar_replay_finished", fields: ["scan": .string(scan)])
        }
        gate.trace.record("calendar_publish_started", keys: pending.map { $0["native_key"].string }, fields: ["scan": .string(scan), "records": .number(Double(pending.count)), "bytes": .number(Double(captureBytes))])
        try offer(pending)
        try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: scan, scanStartedAt: started, scope: ["calendar.calendar", "calendar.event"], completionEvidence: "complete", blobHandles: [:], live: true)
        gate.trace.record("calendar_publish_accepted", fields: ["scan": .string(scan)])
        let persist = forceFull || !pending.isEmpty
        committed = current; forceFull = false
        if scope == nil { lastFullScanStarted = scanNS }
        if persist { persistAccepted(current) }
        gate.trace.record("calendar_scan_complete", fields: ["records": .number(Double(current.count))])
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
        let databaseHints = CalendarDatabaseHints()
        let hints = Task { @AgendaActor [gate] in
            for await delivered in databaseHints.changes {
                gate.calendarHint(delivered)
            }
        }
        defer { hints.cancel(); databaseHints.close() }
        try await scan(into: sink, live: false)
        let periodic = Task { @AgendaActor in
            var day = Calendar.current.startOfDay(for: Date())
            var last = Date()
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                let today = Calendar.current.startOfDay(for: Date())
                if today != day || Date().timeIntervalSince(last) >= 900 { day = today; last = Date(); self.gate.reconcile() }
            }
        }
        defer { periodic.cancel() }
        var handledHint: UInt64 = 0
        for await _ in changes {
            try Task.checkCancellation()
            let hint = gate.calendarDebounceAt
            if hint != handledHint && gate.calendarNotificationAt <= lastFullScanStarted {
                handledHint = hint
                continue
            }
            handledHint = hint
            try await scan(into: sink, live: true)
        }
    }

    private func remember(_ event: EKEvent) {
        lookups[key(event)] = .object(["event_identifier": .string(event.eventIdentifier ?? ""), "calendar": .string(event.calendar.calendarIdentifier), "account": .string(event.calendar.source.sourceIdentifier), "external": .string(event.calendarItemExternalIdentifier == nil ? "" : seriesIdentifier(event)), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null])
    }

    private func lookup(_ native: String) -> EKEvent? {
        guard gate.authorized(.event), let identity = lookups[native] else { return nil }
        func matches(_ event: EKEvent) -> Bool {
            guard event.calendar.calendarIdentifier == identity["calendar"].string, event.calendar.source.sourceIdentifier == identity["account"].string else { return false }
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
                if let event = gate.store.calendarItems(withExternalIdentifier: alias).compactMap({ $0 as? EKEvent }).first(where: matches) { return event }
            }
            let anchor = originals.count > 1 ? originals[1] : original
            if let event = (try? events(DateInterval(start: anchor.addingTimeInterval(-1), end: anchor.addingTimeInterval(1))))?.first(where: matches) { return event }
            if let event = gate.store.event(withIdentifier: identity["event_identifier"].string), event.isDetached, matches(event) { return event }
            return nil
        }
        if let event = gate.store.event(withIdentifier: identity["event_identifier"].string), matches(event) { return event }
        if !identity["external"].string.isEmpty {
            if let event = gate.store.calendarItems(withExternalIdentifier: identity["external"].string).compactMap({ $0 as? EKEvent }).first(where: matches) { return event }
        }
        return nil
    }

    private func knownAbsent(_ native: String) -> Bool {
        guard gate.authorized(.event), let identity = lookups[native], !identity["event_identifier"].string.isEmpty else { return false }
        return gate.store.event(withIdentifier: identity["event_identifier"].string) == nil
    }

    private func fresh(_ native: String) throws -> EKEvent {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        if let event = lookup(native) { return event }
        for chunk in CalendarTime.chunks(Date()) {
            if let event = try events(chunk).first(where: { key($0) == native }) { remember(event); return event }
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
        if !action["native_key"].string.isEmpty {
            let event = try fresh(action["native_key"].string)
            before = data(event)
            _ = try writable(event.calendar.calendarIdentifier)
            guard action["expected_list_key"].string == event.calendar.calendarIdentifier else { throw SourceFailure.conflict }
            var expected = command["expected_state"].object, actual = before.object
            expected.removeValue(forKey: "calendar_id"); actual.removeValue(forKey: "calendar_id")
            guard expected == actual else { throw SourceFailure.conflict }
        }
        if action["op"].string != "calendar.create_calendar" { _ = try writable(action["list_key"].string) }
        else { guard gate.store.defaultCalendarForNewEvents?.allowsContentModifications == true else { throw SourceFailure.unsupported } }
        let prepared = Prepared(nativeState: before, fingerprint: command["request_hash"].string, action: action)
        if action["op"].string != "calendar.delete_event" && action["op"].string != "calendar.create_calendar" {
            let probe = EKEvent(eventStore: gate.store)
            if let event = try? fresh(action["native_key"].string) { probe.startDate = event.startDate; probe.endDate = event.endDate; probe.isAllDay = event.isAllDay; probe.timeZone = event.timeZone }
            try patch(probe, args(action))
        }
        return prepared
    }

    private func patch(_ event: EKEvent, _ args: JSON) throws {
        if let value = args.object["title"] { event.title = value.string }
        if let value = args.object["notes"] { event.notes = value == .null ? nil : value.string }
        if let value = args.object["location"] { event.location = value == .null ? nil : value.string }
        if let value = args.object["url"] { event.url = value == .null ? nil : URL(string: value.string) }
        if let value = args.object["all_day"] { event.isAllDay = value.bool }
        if let value = args.object["time_zone"] {
            guard let zone = TimeZone(identifier: value.string) else { throw SourceFailure.invalid }
            event.timeZone = zone
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
                if item["count"].number > 0 { end = EKRecurrenceEnd(occurrenceCount: Int(item["count"].number)) }
                else if let date = parseTimestamp(item["end"].string) { end = EKRecurrenceEnd(end: date) }
                return EKRecurrenceRule(recurrenceWith: frequency, interval: interval, end: end)
            }
        }
    }

    public func validate(_ prepared: Prepared) async throws {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let action = prepared.action
        if action["op"].string != "calendar.create_calendar" { _ = try writable(action["list_key"].string) }
        if !action["native_key"].string.isEmpty {
            let event = try fresh(action["native_key"].string)
            guard data(event) == prepared.nativeState else { throw SourceFailure.conflict }
            _ = try writable(event.calendar.calendarIdentifier)
        }
    }

    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        try await validate(prepared)
        let action = prepared.action, operation = prepared.action["op"].string
        var native = action["native_key"].string
        if operation == "calendar.create_calendar" {
            guard let source = gate.store.defaultCalendarForNewEvents?.source else { throw SourceFailure.unavailable }
            let calendar = EKCalendar(for: .event, eventStore: gate.store)
            calendar.source = source; calendar.title = args(action)["title"].string
            try gate.store.saveCalendar(calendar, commit: true)
            native = calendar.calendarIdentifier
            pendingScope = pendingScope ?? CalendarScanScope(interval: CalendarTime.window(Date()), calendars: [])
            gate.trace.record("calendar_native_effect", keys: [native], fields: ["operation": .string(operation)])
            guard gate.store.calendar(withIdentifier: native)?.title == calendar.title else { throw SourceFailure.unavailable }
        } else {
            let event = operation == "calendar.create_event" ? EKEvent(eventStore: gate.store) : try fresh(native)
            if operation != "calendar.create_event" { touch(event, future: args(action)["span"].string == "future") }
            event.calendar = try writable(action["list_key"].string)
            let span: EKSpan = args(action)["span"].string == "future" ? .futureEvents : .thisEvent
            if operation == "calendar.delete_event" { try gate.store.remove(event, span: span, commit: true) }
            else { try patch(event, args(action)); try gate.store.save(event, span: span, commit: true); native = key(event) }
            if operation != "calendar.delete_event" { touch(event, future: span == .futureEvents || event.hasRecurrenceRules) }
            gate.trace.record("calendar_native_effect", keys: [native], fields: ["operation": .string(operation)])
            if operation == "calendar.delete_event" {
                guard (try? fresh(native)) == nil else { throw SourceFailure.unavailable }
            } else {
                let observed = try fresh(native)
                guard data(observed) == data(event) else { throw SourceFailure.unavailable }
            }
        }
        commandRevision += 1
        commandNeedsCapture = true
        return .object(["native_key": .string(native), "fingerprint": .string(prepared.fingerprint), "observed": .bool(true), "absent": .bool(operation == "calendar.delete_event")])
    }

    public func confirmAbsence(_ native: String, into sink: any ObservationSink) async throws {
        guard gate.authorized(.event), (try? fresh(native)) == nil, knownAbsent(native) else { throw SourceFailure.conflict }
        let absent: JSON = .object(["kind": .string("absent"), "collection": .string("calendar.event"), "native_key": .string(native), "observed_at": .string(timestamp()), "revalidated": .bool(true)])
        try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: [absent], scanID: UUID().uuidString, scanStartedAt: timestamp(), scope: ["calendar.event"], completionEvidence: "complete", blobHandles: [:], live: true)
    }

    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard gate.authorized(.event) else {return .null}
 let action=evidence["prepared"]["action"]
 let key=action["native_key"].string.isEmpty ? evidence["native_key"].string : action["native_key"].string
 guard !key.isEmpty else {return .null}
        if action["op"].string == "calendar.create_calendar" {
            guard gate.store.calendar(withIdentifier: key)?.title == args(action)["title"].string else { return .null }
        } else if action["op"].string == "calendar.delete_event" {
            guard (try? fresh(key)) == nil, knownAbsent(key) else { return .null }
        } else {
            guard let current = try? fresh(key) else { return .null }
            let expected = args(action).object.filter { !["id", "calendar_id", "span"].contains($0.key) }
            guard expected.allSatisfy({ data(current)[$0.key] == $0.value }) else { return .null }
        }
var result=evidence.object
 result["native_key"] = .string(key);result["observed"] = .bool(true);result["absent"] = .bool(action["op"].string=="calendar.delete_event")
 return .object(result)
    }
}
