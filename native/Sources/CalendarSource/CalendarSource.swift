import Foundation
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@AgendaActor
public final class CalendarSource: Source {
    public nonisolated let name = "calendar"
    public static let operations = ["calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"]
    private let gate: AgendaGate
    private let commandsEnabled: Bool
    public nonisolated init(gate: AgendaGate, commandsEnabled: Bool) {
        self.gate = gate; self.commandsEnabled = commandsEnabled
    }

    public func status() async -> JSON? {
        .object(["authorized": .bool(gate.authorized(.event))])
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
        var row = observation("calendar.event", native, data(event), alias: alias).object
        row["locator"] = locator(event)
        return .object(row)
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
                current["calendar.event/" + native] = eventObservation(event, native: native)
            }
        }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return SourceEnumeration(generation: "eventkit-calendar-v1", rows: current.keys.sorted().compactMap { current[$0] })
    }

    public func revalidate(_ identities: [JSON]) async throws -> [String] {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        return try identities.map { identity in
            guard identity["collection"].string == "calendar.calendar" else { throw SourceFailure.invalid }
            return gate.store.calendar(withIdentifier: identity["native_key"].string) == nil ? RevalidationState.missing : RevalidationState.present
        }
    }

    public nonisolated static func canMaterialize(_ event: EKEvent) -> Bool {
        guard event.startDate != nil, event.endDate != nil,
              let calendar = event.calendar, calendar.source != nil else { return false }
        return true
    }


    private func locator(_ event: EKEvent) -> JSON {
        .object(["event_identifier": .string(event.eventIdentifier ?? ""), "calendar": .string(event.calendar?.calendarIdentifier ?? ""), "account": .string(event.calendar?.source?.sourceIdentifier ?? ""), "external": .string(event.calendarItemExternalIdentifier == nil ? "" : seriesIdentifier(event)), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null])
    }

    private func candidate(_ event: EKEvent) -> JSON {
        guard Self.canMaterialize(event), let calendar = event.calendar, let account = calendar.source else { return .object(["can_materialize": .bool(false)]) }
        let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
        return .object(["can_materialize": .bool(true), "key": .string(key(event)), "calendar": .string(calendar.calendarIdentifier), "account": .string(account.sourceIdentifier), "series": .string(seriesIdentifier(event)), "occurrence": event.occurrenceDate.map { .string(timestamp($0)) } ?? .null, "occurrence_civil": event.occurrenceDate.map { .string(CalendarTime.floating($0, zone: .current)) } ?? .null, "all_day": .bool(event.isAllDay), "detached": .bool(event.isDetached), "start": .string(event.isAllDay ? CalendarTime.floating(event.startDate, zone: zone) : timestamp(event.startDate)), "end": .string(event.isAllDay ? CalendarTime.floating(event.endDate, zone: zone) : timestamp(event.endDate)), "time_zone": event.timeZone.map { .string($0.identifier) } ?? .null, "locator": locator(event)])
    }

    private func answering(_ query: JSON) throws -> [EKEvent] {
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        switch query["via"].string {
        case "identifier":
            guard !query["identifier"].string.isEmpty, let event = gate.store.event(withIdentifier: query["identifier"].string) else { return [] }
            return [event]
        case "external":
            return gate.store.calendarItems(withExternalIdentifier: query["external"].string).compactMap { $0 as? EKEvent }
        case "window":
            let interval = DateInterval(start: Date(timeIntervalSince1970: query["start"].number), end: Date(timeIntervalSince1970: query["end"].number))
            let found = try events(interval, calendars: query["calendar"].string.isEmpty ? nil : [query["calendar"].string])
            return query["key"].string.isEmpty ? found : found.filter { !Self.canMaterialize($0) || key($0) == query["key"].string }
        default:
            throw SourceFailure.invalid
        }
    }

    public func candidates(_ query: JSON) throws -> [JSON] {
        try answering(query).map(candidate)
    }

    public func account(_ calendar: String) -> JSON {
        .object(["account": (gate.store.calendar(withIdentifier: calendar)?.source?.sourceIdentifier).map(JSON.string) ?? .null])
    }

    private func selected(_ selector: JSON) throws -> EKEvent {
        guard let event = try answering(selector["query"]).first(where: { Self.canMaterialize($0) && key($0) == selector["key"].string }) else { throw SourceFailure.conflict }
        return event
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
        var savedTitle: JSON = .null
        var observedTitle: JSON = .null
        var existsAfter = false
        var locator: JSON = .null
    }

    private func perform(_ action: JSON, selector: JSON) async throws -> Performed {
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
            let event = operation == "calendar.create_event" ? EKEvent(eventStore: gate.store) : try selected(selector)
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
            if operation != "calendar.delete_event", Self.canMaterialize(event) { performed.locator = locator(event) }
            return performed
        }
        return performed
    }


    public func primitive(_ operation: String, args request: JSON) async throws -> JSON {
        guard commandsEnabled else { throw SourceFailure.unsupported }
        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }
        let native = request["native_key"].string
        switch operation {
        case "calendar.candidates":
            return .object(["items": .array(try candidates(request["query"]))])
        case "calendar.account":
            return account(request["calendar"].string)
        case "calendar.read_event":
            let event: EKEvent
            do { event = try selected(request["selector"]) } catch SourceFailure.conflict { return .object(["exists": .bool(false), "known_absent": .bool(false)]) }
            return .object(["exists": .bool(true), "key": .string(key(event)), "writable": .bool(event.calendar.allowsContentModifications && !event.calendar.isSubscribed), "calendar_key": .string(event.calendar.calendarIdentifier), "all_day": .bool(event.isAllDay), "data": data(event), "native_result": nativeResult(event)])
        case "calendar.read_calendar":
            guard !native.isEmpty, let calendar = gate.store.calendar(withIdentifier: native) else { return .object(["exists": .bool(false)]) }
            return .object(["exists": .bool(true), "writable": .bool(calendar.allowsContentModifications && !calendar.isSubscribed), "data": calendarData(calendar)])
        case "calendar.can_create_calendar":
            return .object(["allowed": .bool(gate.store.defaultCalendarForNewEvents?.allowsContentModifications == true)])
        case "calendar.goal":
            let existing = native.isEmpty ? EKEvent(eventStore: gate.store) : try selected(request["selector"])
            let calendar = try writable(request["list_key"].string)
            let goal = try requestedGoal(existing, request["calendar_args"], calendar: calendar, prior: request["prior"])
            return .object(["goal": goal, "all_day": .bool(existing.isAllDay)])
        case "calendar.write":
            let action: JSON = .object(["op": request["op"], "native_key": request["native_key"], "list_key": request["list_key"], "calendar_args": request["calendar_args"]])
            let performed = try await perform(action, selector: request["selector"])
            var result: [String: JSON] = ["native_key": .string(performed.native), "exists_after": .bool(performed.existsAfter)]
            if request["op"].string == "calendar.create_calendar" {
                result["saved_title"] = performed.savedTitle; result["observed_title"] = performed.observedTitle; result["actual"] = performed.calendarResult
            } else if request["op"].string != "calendar.delete_event" {
                result["goal"] = performed.goal; result["all_day"] = .bool(performed.allDay); result["locator"] = performed.locator
            }
            return .object(result)
        default:
            throw SourceFailure.unsupported
        }
    }


}
