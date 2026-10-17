import Foundation
@preconcurrency import EventKit
import Darwin

@MainActor
struct CalendarTruth {
    static func run(_ verb: String, _ request: [String: Any]) throws -> [String: Any] {
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { throw TruthFailure.permissionMissing }
        guard verb == "fixture" || verb == "observe" else { throw TruthFailure.invalid }
        let store = EKEventStore()
        let action = request["action"] as? String ?? ""
        let tag = request["tag"] as? String ?? ""
        guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), tag.count > 11, !tag.contains("\n") else { throw TruthFailure.invalid }
        if action == "readonly" {
            guard verb == "observe" else { throw TruthFailure.invalid }
            let calendars = store.calendars(for: .event).filter { $0.isSubscribed }
            return ["status": "verified", "calendars": calendars.map { ["id": $0.calendarIdentifier, "writable": false] }]
        }
        var calendars = store.calendars(for: .event).filter { $0.title == "Bridge Test" }
        if let key = request["calendar_id"] as? String { calendars = calendars.filter { $0.calendarIdentifier == key } }
        if calendars.isEmpty && verb == "fixture" && ["create", "create_calendar"].contains(action) {
            guard let source = store.defaultCalendarForNewEvents?.source else { throw TruthFailure.unavailable }
            let calendar = EKCalendar(for: .event, eventStore: store)
            calendar.title = "Bridge Test"; calendar.source = source
            try store.saveCalendar(calendar, commit: true)
            calendars = [calendar]
        }
        var bounds = Calendar(identifier: .gregorian)
        bounds.timeZone = .current
        let today = bounds.startOfDay(for: Date())
        let left = bounds.date(byAdding: .day, value: -730, to: today)!
        let right = bounds.date(byAdding: .day, value: 730, to: today)!
        var events: [EKEvent] = []
        var keys = Set<String>()
        var start = left
        while start < right {
            let end = min(bounds.date(byAdding: .year, value: 1, to: start)!, right)
            if !calendars.isEmpty {
                let predicate = store.predicateForEvents(withStart: start, end: end, calendars: calendars)
                for event in store.events(matching: predicate) {
                    let key = event.calendarItemIdentifier + "|" + String(event.occurrenceDate?.timeIntervalSince1970 ?? 0)
                    if keys.insert(key).inserted { events.append(event) }
                }
            }
            start = end
        }
        let selected = events.filter { event in
            (event.title ?? "").contains(tag) && ((request["title"] as? String).map { event.title == $0 } ?? true) && ((request["occurrence_date"] as? String).map { stamp(event.occurrenceDate ?? event.startDate) == $0 } ?? true)
        }.sorted { $0.startDate < $1.startDate }
        if verb == "fixture" {
            switch action {
            case "create_calendar":
                guard let calendar = calendars.first else { throw TruthFailure.unavailable }
                return ["status": "verified", "id": calendar.calendarIdentifier]
            case "create":
                guard let calendar = calendars.first, calendar.allowsContentModifications, let title = request["title"] as? String, title.contains(tag) else { throw TruthFailure.invalid }
                let event = EKEvent(eventStore: store)
                event.calendar = calendar; event.title = title
                event.isAllDay = request["all_day"] as? Bool ?? false
                event.timeZone = TimeZone(identifier: request["time_zone"] as? String ?? "America/New_York")
                event.startDate = try date(request["start"] as? String ?? "", allDay: event.isAllDay, zone: event.isAllDay ? .current : (event.timeZone ?? .current))
                event.endDate = try date(request["end"] as? String ?? "", allDay: event.isAllDay, zone: event.isAllDay ? .current : (event.timeZone ?? .current))
                if let recurrence = request["recurrence"] as? [[String: Any]], let rule = recurrence.first {
                    let frequency: EKRecurrenceFrequency = rule["frequency"] as? String == "weekly" ? .weekly : .daily
                    event.addRecurrenceRule(EKRecurrenceRule(recurrenceWith: frequency, interval: rule["interval"] as? Int ?? 1, end: EKRecurrenceEnd(occurrenceCount: rule["count"] as? Int ?? 3)))
                }
                try store.save(event, span: .thisEvent, commit: true)
                return ["status": "verified", "id": event.calendarItemIdentifier, "effect_uptime_ns": clock_gettime_nsec_np(CLOCK_UPTIME_RAW)]
            case "modify", "move":
                guard let index = request["index"] as? Int, selected.indices.contains(index) else { throw TruthFailure.invalid }
                let event = selected[index]
                guard event.calendar.title == "Bridge Test", event.calendar.allowsContentModifications else { throw TruthFailure.invalid }
                if let title = request["new_title"] as? String { guard title.contains(tag) else { throw TruthFailure.invalid }; event.title = title }
                if let value = request["start"] as? String {
                    let duration = event.endDate.timeIntervalSince(event.startDate)
                    event.startDate = try date(value, allDay: event.isAllDay, zone: event.isAllDay ? .current : (event.timeZone ?? .current))
                    event.endDate = event.startDate.addingTimeInterval(duration)
                }
                try store.save(event, span: request["span"] as? String == "future" ? .futureEvents : .thisEvent, commit: true)
            case "cleanup", "delete":
                var seen = Set<String>()
                for event in selected {
                    guard event.calendar.title == "Bridge Test", (event.title ?? "").contains(tag), event.calendar.allowsContentModifications else { throw TruthFailure.invalid }
                    let identity = event.calendarItemIdentifier
                    if seen.insert(identity).inserted { try store.remove(event, span: .futureEvents, commit: true) }
                }
            case "cleanup_calendar":
                guard let key = request["calendar_id"] as? String, let calendar = calendars.first(where: { $0.calendarIdentifier == key }), events.allSatisfy({ ($0.title ?? "").contains(tag) }) else { throw TruthFailure.invalid }
                try store.removeCalendar(calendar, commit: true)
            default: throw TruthFailure.invalid
            }
            return ["status": "verified", "effect_uptime_ns": clock_gettime_nsec_np(CLOCK_UPTIME_RAW)]
        }
        let records: [[String: Any]] = selected.map { event in
            let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
            var row: [String: Any] = ["id": event.calendarItemIdentifier, "external_id": event.calendarItemExternalIdentifier ?? "", "calendar_id": event.calendar.calendarIdentifier, "title": event.title ?? "", "start": event.isAllDay ? floating(event.startDate, zone: zone) : stamp(event.startDate), "end": event.isAllDay ? floating(event.endDate, zone: zone) : stamp(event.endDate), "all_day": event.isAllDay, "time_zone": event.timeZone?.identifier ?? "", "is_exception": event.isDetached, "occurrence_date": event.occurrenceDate.map { event.isAllDay ? floating($0,zone:.current) : stamp($0) } ?? "", "recurring": event.hasRecurrenceRules]
            row["notes"] = event.notes.map { $0 as Any } ?? NSNull()
            row["location"] = event.location.map { $0 as Any } ?? NSNull()
            row["url"] = event.url.map { $0.absoluteString as Any } ?? NSNull()
            row["availability"] = event.availability.rawValue
            row["status"] = event.status.rawValue
            row["attendees"] = (event.attendees ?? []).map { person -> [String: Any] in
                ["name": person.name.map { $0 as Any } ?? NSNull(), "url": person.url.absoluteString, "status": person.participantStatus.rawValue, "role": person.participantRole.rawValue, "type": person.participantType.rawValue]
            }
            row["alarms"] = (event.alarms ?? []).map { alarm -> [String: Any] in
                ["absolute_date": alarm.absoluteDate.map { stamp($0) as Any } ?? NSNull(), "relative_offset": alarm.relativeOffset]
            }
            return row
        }
        return ["status": "verified", "calendars": calendars.map { calendar in
            let names = ["local", "caldav", "exchange", "subscribed", "birthday"]
            let index = Int(calendar.type.rawValue)
            return ["id": calendar.calendarIdentifier, "title": "Bridge Test", "writable": calendar.allowsContentModifications && !calendar.isSubscribed, "account": calendar.source.sourceIdentifier, "color": calendar.cgColor.map { String(describing: $0.components ?? []) } ?? "", "type": calendar.isSubscribed ? "subscribed" : (names.indices.contains(index) ? names[index] : "unknown")] as [String: Any]
        }, "records": records]
    }

    private static func stamp(_ date: Date) -> String {
        let format = ISO8601DateFormatter(); format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return format.string(from: date)
    }
    private static func floating(_ date: Date, zone: TimeZone) -> String {
        let format = DateFormatter(); format.locale = Locale(identifier: "en_US_POSIX"); format.timeZone = zone; format.dateFormat = "yyyy-MM-dd"
        return format.string(from: date)
    }
    private static func date(_ value: String, allDay: Bool, zone: TimeZone) throws -> Date {
        if allDay {
            let format = DateFormatter(); format.locale = Locale(identifier: "en_US_POSIX"); format.timeZone = zone; format.dateFormat = "yyyy-MM-dd"; format.isLenient = false
            guard let result = format.date(from: value), format.string(from: result) == value else { throw TruthFailure.invalid }
            return result
        }
        let format = ISO8601DateFormatter(); format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let result = format.date(from: value) ?? ISO8601DateFormatter().date(from: value) else { throw TruthFailure.invalid }
        return result
    }
}
