import Foundation
import CryptoKit
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@MainActor
public final class CalendarSource: Source {
    public nonisolated let name = "calendar"
    public static let operations = ["calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"]
    public static let coverage: JSON = .object(["fields": .string("complete"), "missing": .array([])])
    private let gate: AgendaGate
    private let baselineURL: URL
    private let commandsEnabled: Bool
    private let lookupURL: URL
    private var lookups: [String: JSON] = [:]
    private var committed: [String: JSON] = [:]
    private var scanning = false
    private var forceFull = true

    public init(gate: AgendaGate, home: URL, commandsEnabled: Bool) throws {
        self.gate = gate; self.commandsEnabled = commandsEnabled
        baselineURL = home.appendingPathComponent("calendar-baseline.json")
        lookupURL = home.appendingPathComponent("calendar-lookups.json")
        if FileManager.default.fileExists(atPath: lookupURL.path) { lookups = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: lookupURL)) }
        if FileManager.default.fileExists(atPath: baselineURL.path) {
            committed = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: baselineURL))
        }
    }

    public func open() async -> SourceReadiness {
        let authorized = gate.authorized(.event)
        return SourceReadiness(state: authorized ? "ready" : "permission_missing", coverage: Self.coverage, capabilities: authorized && commandsEnabled ? Self.operations : [], reason: authorized ? "" : "calendar_full_access_required")
    }

    private func key(_ event: EKEvent) -> String {
        CalendarTime.identity(event.calendarItemIdentifier, occurrence: event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate : nil, floatingZone: event.isAllDay ? .current : nil)
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
            fields["series_id"] = .string(event.calendarItemExternalIdentifier ?? event.calendarItemIdentifier)
            fields["occurrence_date"] = event.occurrenceDate.map { .string(event.isAllDay ? CalendarTime.floating($0, zone: .current) : timestamp($0)) } ?? .null
        }
        var nativeRules = event.recurrenceRules
        if nativeRules == nil && event.isDetached {
            let external = event.calendarItemExternalIdentifier ?? event.calendarItemIdentifier
            var candidates = gate.store.calendarItems(withExternalIdentifier: external).compactMap { $0 as? EKEvent }
            if let master = gate.store.calendarItem(withIdentifier: event.calendarItemIdentifier) as? EKEvent { candidates.append(master) }
            let anchor = event.occurrenceDate ?? event.startDate!
            let interval = DateInterval(start: anchor.addingTimeInterval(-180 * 86400), end: anchor.addingTimeInterval(180 * 86400))
            candidates += (try? events(interval)) ?? []
            nativeRules = candidates.first(where: {
                $0.calendar.calendarIdentifier == event.calendar.calendarIdentifier && $0.calendar.source.sourceIdentifier == event.calendar.source.sourceIdentifier && (($0.calendarItemExternalIdentifier ?? $0.calendarItemIdentifier) == external || $0.calendarItemIdentifier == event.calendarItemIdentifier) && $0.hasRecurrenceRules
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

    private func events(_ interval: DateInterval) throws -> [EKEvent] {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let predicate = gate.store.predicateForEvents(withStart: interval.start, end: interval.end, calendars: nil)
        let events = gate.store.events(matching: predicate)
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return events
    }

    private func save(_ values: [String: JSON]) throws {
        try JSONEncoder().encode(lookups).write(to: lookupURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: lookupURL.path)
        try JSONEncoder().encode(values).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
    }

    private func hash(_ value: JSON) throws -> Data {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        return Data(SHA256.hash(data: try encoder.encode(value)))
    }

    private func scan(into sink: any ObservationSink, live: Bool) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(10)) }
        scanning = true
        defer { scanning = false }
        let started = timestamp()
        let now = Date()
        let bounds = CalendarTime.window(now)
        var current: [String: JSON] = [:]
        var pending: [JSON] = []
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        for calendar in gate.store.calendars(for: .event) {
            let value = observation("calendar.calendar", calendar.calendarIdentifier, calendarData(calendar))
            current["calendar.calendar/" + calendar.calendarIdentifier] = value
            if forceFull || value["data"] != committed["calendar.calendar/" + calendar.calendarIdentifier]?["data"] { pending.append(value) }
        }
        for (index, chunk) in CalendarTime.chunks(now).enumerated() {
            for event in try events(chunk) {
                let native = key(event)
                remember(event)
                let identity = "calendar.event/" + native
                if current[identity] != nil { continue }
                let occurrence = event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate.map { "|" + (event.isAllDay ? CalendarTime.floating($0, zone: .current) : timestamp($0)) } ?? "" : ""
                let alias = event.calendarItemExternalIdentifier.map { event.calendar.source.sourceIdentifier + "/" + event.calendar.calendarIdentifier + "/" + $0 + occurrence } ?? ""
                let value = observation("calendar.event", native, data(event), alias: alias)
                current[identity] = value
                let changed = try hash(value["data"]) != hash(committed[identity]?["data"] ?? .null)
                if forceFull || changed || value["scoped_alias"] != committed[identity]?["scoped_alias"] { pending.append(value) }
            }
            guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
            try save(committed.merging(current) { _, new in new })
            try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: "", scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: [:], live: live || index == 0)
            pending = []
        }
        for (identity, previous) in committed where current[identity] == nil {
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
                } else if !knownAbsent(native) {
                    current[identity] = previous
                    continue
                }
            }
            absent["kind"] = .string(evicted ? "evict" : "absent")
            pending.append(.object(absent))
        }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        try await sink.emit(source: name, generation: "eventkit-calendar-v1", observations: pending, scanID: UUID().uuidString, scanStartedAt: started, scope: ["calendar.calendar", "calendar.event"], completionEvidence: "complete", blobHandles: [:], live: live)
        try save(current); committed = current; forceFull = false
 let readiness=await open()
 try await sink.sourceState(source:name,state:readiness.state,coverage:readiness.coverage,capabilities:readiness.capabilities,reason:readiness.reason)
    }

    public func reconcile(into sink: any ObservationSink) async throws {
        forceFull = true
        try await scan(into: sink, live: true)
    }

    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        forceFull = true
        let changes = gate.changes()
        try await scan(into: sink, live: false)
        let periodic = Task { @MainActor in
            var day = Calendar.current.startOfDay(for: Date())
            var last = Date()
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                let today = Calendar.current.startOfDay(for: Date())
                if today != day || Date().timeIntervalSince(last) >= 900 { day = today; last = Date(); self.gate.reconcile() }
            }
        }
        defer { periodic.cancel() }
        for await _ in changes { try Task.checkCancellation(); try await scan(into: sink, live: true) }
    }

    private func remember(_ event: EKEvent) {
        lookups[key(event)] = .object(["event_identifier": .string(event.eventIdentifier ?? ""), "calendar": .string(event.calendar.calendarIdentifier), "account": .string(event.calendar.source.sourceIdentifier), "external": .string(event.calendarItemExternalIdentifier ?? ""), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null])
    }

    private func lookup(_ native: String) -> EKEvent? {
        guard gate.authorized(.event), let identity = lookups[native] else { return nil }
        func matches(_ event: EKEvent) -> Bool {
            guard event.calendar.calendarIdentifier == identity["calendar"].string, event.calendar.source.sourceIdentifier == identity["account"].string else { return false }
            if key(event) == native { return true }
            guard !identity["external"].string.isEmpty, event.calendarItemExternalIdentifier == identity["external"].string else { return false }
            return (event.occurrenceDate.map { JSON.string(timestamp($0)) } ?? .null) == identity["occurrence"]
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
            guard gate.store.calendar(withIdentifier: native)?.title == calendar.title else { throw SourceFailure.unavailable }
        } else {
            let event = operation == "calendar.create_event" ? EKEvent(eventStore: gate.store) : try fresh(native)
            event.calendar = try writable(action["list_key"].string)
            let span: EKSpan = args(action)["span"].string == "future" ? .futureEvents : .thisEvent
            if operation == "calendar.delete_event" { try gate.store.remove(event, span: span, commit: true) }
            else { try patch(event, args(action)); try gate.store.save(event, span: span, commit: true); native = key(event) }
            if operation == "calendar.delete_event" {
                guard (try? fresh(native)) == nil else { throw SourceFailure.unavailable }
            } else {
                let observed = try fresh(native)
                guard data(observed) == data(event) else { throw SourceFailure.unavailable }
            }
        }
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
