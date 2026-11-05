import Foundation
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@AgendaActor
public final class CalendarSource: Source {
    public nonisolated let name = "calendar"
    public static let operations = ["calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"]
    public static let coverage: JSON = .object(["operations": CalendarParity.coverage, "fields": .string("partial"), "missing": .array(["travel_time: positive native units unverified", "structured_location: coordinate units unverified", "detached store identities", "calendar attachment derivatives", "start location, proposals, comments and extra store alarms: positive native fixtures pending"].map(JSON.string))])
    private let gate: AgendaGate
    private let commandsEnabled: Bool
    private let lookupURL: URL
    private var lookups: [String: JSON] = [:]
    private var savedLookups: [String: JSON] = [:]



    public nonisolated init(gate: AgendaGate, home: URL, commandsEnabled: Bool) throws {
        self.gate = gate; self.commandsEnabled = commandsEnabled
        lookupURL = home.appendingPathComponent("calendar-lookups.json")
        if FileManager.default.fileExists(atPath: lookupURL.path) { lookups = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: lookupURL)) }
        savedLookups = lookups
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
        var fields: [String: JSON] = ["calendar_id": ref(event.calendar), "title": .string(event.title ?? ""), "notes": event.notes.map(JSON.string) ?? .null, "location": event.location.map(JSON.string) ?? .null, "url": event.url.map { .string($0.absoluteString) } ?? .null, "start": .string(event.isAllDay ? CalendarTime.floating(event.startDate, zone: zone) : timestamp(event.startDate)), "end": .string(event.isAllDay ? CalendarTime.floating(event.endDate, zone: zone) : timestamp(event.endDate)), "all_day": .bool(event.isAllDay), "time_zone": event.timeZone.map { .string($0.identifier) } ?? .null, "is_exception": .bool(event.isDetached), "availability": .number(Double(event.availability.rawValue)), "status": .number(Double(event.status.rawValue)), "attendees": .array((event.attendees ?? []).map { .object(["name": $0.name.map(JSON.string) ?? .null, "url": .string($0.url.absoluteString), "status": .number(Double($0.participantStatus.rawValue)), "role": .number(Double($0.participantRole.rawValue)), "type": .number(Double($0.participantType.rawValue))]) }), "alarms": .array((event.alarms ?? []).map { .object(["absolute_date": $0.absoluteDate.map { .string(timestamp($0)) } ?? .null, "relative_offset": .number($0.relativeOffset)]) })]
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
        let selected = calendars.map { identifiers in gate.store.calendars(for: .event).filter { identifiers.contains($0.calendarIdentifier) } }
        if let selected, selected.isEmpty { return [] }
        let predicate = gate.store.predicateForEvents(withStart: interval.start, end: interval.end, calendars: selected)
        let events = gate.store.events(matching: predicate)
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return events
    }




    private func saveLookups() throws {
        guard lookups != savedLookups else { return }
        try JSONEncoder().encode(lookups).write(to: lookupURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: lookupURL.path)
        savedLookups = lookups
    }





    public func enumerate() async throws -> SourceEnumeration {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        var current: [String: JSON] = [:]
        for calendar in gate.store.calendars(for: .event) {
            current["calendar.calendar/" + calendar.calendarIdentifier] = observation("calendar.calendar", calendar.calendarIdentifier, calendarData(calendar))
        }
        for chunk in CalendarTime.chunks(Date()) {
            for event in try events(chunk) {
                try Task.checkCancellation()
                guard Self.canMaterialize(event) else { throw SourceFailure.unavailable }
                let native = key(event)
                guard current["calendar.event/" + native] == nil else { continue }
                try remember(event)
                current["calendar.event/" + native] = eventObservation(event, native: native)
            }
        }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        try saveLookups()
        return SourceEnumeration(generation: "eventkit-calendar-v1", rows: current.keys.sorted().compactMap { current[$0] })
    }

    public func revalidate(_ identities: [JSON]) async throws -> [String] {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let bounds = CalendarTime.window(Date())
        var states: [String] = []
        for identity in identities {
            let native = identity["native_key"].string
            switch identity["collection"].string {
            case "calendar.calendar":
                states.append(gate.store.calendar(withIdentifier: native) == nil ? RevalidationState.missing : RevalidationState.present)
            case "calendar.event":
                if let found = try lookup(native) {
                    let value = data(found)
                    let zone = value["all_day"].bool ? TimeZone.current : (TimeZone(identifier: value["time_zone"].string) ?? .current)
                    guard let start = try? CalendarTime.date(value["start"].string, allDay: value["all_day"].bool, zone: zone), let end = try? CalendarTime.date(value["end"].string, allDay: value["all_day"].bool, zone: zone) else { states.append(RevalidationState.present); continue }
                    states.append(start >= bounds.end || end <= bounds.start ? RevalidationState.outsideScope : RevalidationState.present)
                } else {
                    let value = identity["data"]
                    let original = value["occurrence_date"].string.isEmpty ? value["start"].string : value["occurrence_date"].string
                    guard let date = try? CalendarTime.date(original, allDay: value["all_day"].bool, zone: .current) else { states.append(RevalidationState.present); continue }
                    states.append(date < bounds.start || date >= bounds.end ? RevalidationState.outsideScope : RevalidationState.missing)
                }
            default: throw SourceFailure.invalid
            }
            guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        }
        return states
    }


    public nonisolated static func canMaterialize(_ event: EKEvent) -> Bool {
        guard event.startDate != nil, event.endDate != nil,
              let calendar = event.calendar, calendar.source != nil else { return false }
        return true
    }


    private func remember(_ event: EKEvent) throws {
        guard Self.canMaterialize(event), let calendar = event.calendar, let account = calendar.source else { throw SourceFailure.unavailable }
        lookups[key(event)] = .object(["event_identifier": .string(event.eventIdentifier ?? ""), "calendar": .string(calendar.calendarIdentifier), "account": .string(account.sourceIdentifier), "external": .string(event.calendarItemExternalIdentifier == nil ? "" : seriesIdentifier(event)), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null])
    }

    private func lookup(_ native: String) throws -> EKEvent? {
        guard gate.authorized(.event), let identity = lookups[native] else { return nil }
        func matches(_ event: EKEvent) throws -> Bool {
            guard Self.canMaterialize(event), let calendar = event.calendar, let account = calendar.source else { throw SourceFailure.unavailable }
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

    private func requestedGoal(_ event: EKEvent, _ request: JSON, calendar: EKCalendar, prior: JSON = .null) throws -> JSON {
        let probe = EKEvent(eventStore: gate.store)
        probe.calendar = calendar
        probe.isAllDay = event.isAllDay
        probe.timeZone = event.timeZone
        probe.startDate = event.startDate
        probe.endDate = event.endDate
        if prior != .null {
            let values = prior.object.filter { ["start", "end", "all_day", "time_zone"].contains($0.key) }
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



    private struct Performed {
        var native: String
        var calendarResult: JSON = .null
        var goal: JSON = .null
        var allDay = false
        var actual: JSON = .null
        var savedTitle: JSON = .null
        var observedTitle: JSON = .null
        var existsAfter = false
    }

    private func perform(_ action: JSON) async throws -> Performed {
        let operation = action["op"].string
        var performed = Performed(native: action["native_key"].string)
        if operation == "calendar.create_calendar" {
            guard let source = gate.store.defaultCalendarForNewEvents?.source else { throw SourceFailure.unavailable }
            let calendar = EKCalendar(for: .event, eventStore: gate.store)
            calendar.source = source; calendar.title = args(action)["title"].string
            try gate.store.saveCalendar(calendar, commit: true)
            performed.native = calendar.calendarIdentifier
            let observed = gate.store.calendar(withIdentifier: performed.native)
            performed.savedTitle = .string(calendar.title)
            performed.observedTitle = observed.map { .string($0.title) } ?? .null
            performed.calendarResult = observed.map(calendarData) ?? .null
            performed.existsAfter = observed != nil
        } else {
            let event = operation == "calendar.create_event" ? EKEvent(eventStore: gate.store) : try fresh(performed.native)
            event.calendar = try writable(action["list_key"].string)
            let span: EKSpan = args(action)["span"].string == "future" ? .futureEvents : .thisEvent
            performed.goal = operation == "calendar.delete_event" ? JSON.null : try requestedGoal(event, args(action), calendar: event.calendar)
            performed.allDay = args(action).object["all_day"]?.bool ?? event.isAllDay
            if operation == "calendar.delete_event" {
                try gate.store.remove(event, span: span, commit: true)
            } else {
                try patch(event, args(action))
                try gate.store.save(event, span: span, commit: true)
                if Self.canMaterialize(event) { performed.native = key(event) }
            }
            if operation == "calendar.delete_event" {
                performed.existsAfter = (try? fresh(performed.native)) != nil
            } else {
                let observed = try? fresh(performed.native)
                performed.actual = observed.map(nativeResult) ?? .null
                if let observed { performed.native = key(observed) }
                performed.existsAfter = observed != nil
            }
            return performed
        }
        return performed
    }


    public func primitive(_ operation: String, args request: JSON) async throws -> JSON {
        guard commandsEnabled else { throw SourceFailure.unsupported }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let native = request["native_key"].string
        switch operation {
        case "calendar.read_event":
            guard !native.isEmpty else { return .object(["exists": .bool(false), "known_absent": .bool(false)]) }
            let event: EKEvent
            do { event = try fresh(native) } catch SourceFailure.conflict { return .object(["exists": .bool(false), "known_absent": .bool(knownAbsent(native))]) }
            return .object(["exists": .bool(true), "writable": .bool(event.calendar.allowsContentModifications && !event.calendar.isSubscribed), "calendar_key": .string(event.calendar.calendarIdentifier), "all_day": .bool(event.isAllDay), "data": data(event), "native_result": nativeResult(event)])
        case "calendar.read_calendar":
            guard !native.isEmpty, let calendar = gate.store.calendar(withIdentifier: native) else { return .object(["exists": .bool(false)]) }
            return .object(["exists": .bool(true), "writable": .bool(calendar.allowsContentModifications && !calendar.isSubscribed), "data": calendarData(calendar)])
        case "calendar.can_create_calendar":
            return .object(["allowed": .bool(gate.store.defaultCalendarForNewEvents?.allowsContentModifications == true)])
        case "calendar.goal":
            let existing = native.isEmpty ? EKEvent(eventStore: gate.store) : try fresh(native)
            let calendar = try writable(request["list_key"].string)
            let goal = try requestedGoal(existing, request["calendar_args"], calendar: calendar, prior: request["prior"])
            return .object(["goal": goal, "all_day": .bool(existing.isAllDay)])
        case "calendar.write":
            let action: JSON = .object(["op": request["op"], "native_key": request["native_key"], "list_key": request["list_key"], "calendar_args": request["calendar_args"]])
            let performed = try await perform(action)
            var result: [String: JSON] = ["native_key": .string(performed.native), "exists_after": .bool(performed.existsAfter)]
            if request["op"].string == "calendar.create_calendar" {
                result["saved_title"] = performed.savedTitle; result["observed_title"] = performed.observedTitle; result["actual"] = performed.calendarResult
            } else if request["op"].string != "calendar.delete_event" {
                result["goal"] = performed.goal; result["all_day"] = .bool(performed.allDay); result["actual"] = performed.actual
            }
            return .object(result)
        default:
            throw SourceFailure.unsupported
        }
    }


}
