import Foundation
@preconcurrency import EventKit
import Darwin

@MainActor
struct CalendarTruth {
    private struct Ownership: Codable {
        var calendars: [String: String] = [:]
        var events: [String: [String]] = [:]
        var calendarReceipts: [String: String]?
        var canonicalCalendar: String?
    }
    private static var ownershipURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/calendar-fixture-ownership.json")
    }
    private static func ownership() throws -> Ownership {
        guard FileManager.default.fileExists(atPath: ownershipURL.path) else { return Ownership() }
        return try JSONDecoder().decode(Ownership.self, from: Data(contentsOf: ownershipURL))
    }
    private static func saveOwnership(_ value: Ownership) throws {
        try FileManager.default.createDirectory(at: ownershipURL.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try JSONEncoder().encode(value).write(to: ownershipURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: ownershipURL.path)
    }
    private static func remember(_ event: EKEvent, tag: String) throws {
        guard event.calendar.title == "Bridge Test", (event.title ?? "").hasPrefix(tag), let identifier = event.eventIdentifier else { throw TruthFailure.invalid }
        var value = try ownership()
        if !(value.events[tag] ?? []).contains(identifier) { value.events[tag, default: []].append(identifier) }
        try saveOwnership(value)
    }
    private static func consolidate(_ store: EKEventStore, proof: [String: Any]) throws -> [String: Any] {
        guard proof["all_dates"] as? Bool == true, proof["read_only"] as? Bool == true, let generated = proof["generated_at"] as? Double, let counts = proof["counts"] as? [String: NSNumber], generated.isFinite, (0...10).contains(Date().timeIntervalSince1970 - generated) else { throw TruthFailure.invalid }
        var owned = try ownership()
        let calendars = store.calendars(for: .event).filter { $0.title == "Bridge Test" }.sorted {
            if ($0.calendarIdentifier == owned.canonicalCalendar) != ($1.calendarIdentifier == owned.canonicalCalendar) { return $0.calendarIdentifier == owned.canonicalCalendar }
            return $0.calendarIdentifier < $1.calendarIdentifier
        }
        guard let retained = calendars.first else { return ["before": 0, "after": 0, "removed": 0, "all_dates": true, "empty_only": true] }
        owned.canonicalCalendar = retained.calendarIdentifier
        try saveOwnership(owned)
        var removed = 0
        for calendar in calendars.dropFirst() {
            guard calendar.title == "Bridge Test", calendar.allowsContentModifications, owned.calendars[calendar.calendarIdentifier] != nil else { continue }
            guard counts[calendar.calendarIdentifier]?.int64Value == 0 else { continue }
            try store.removeCalendar(calendar, commit: true)
            owned.calendars.removeValue(forKey: calendar.calendarIdentifier)
            owned.calendarReceipts?.removeValue(forKey: calendar.calendarIdentifier)
            try saveOwnership(owned)
            removed += 1
        }
        return ["before": calendars.count, "after": calendars.count - removed, "removed": removed, "all_dates": true, "empty_only": true]
    }
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
        if action == "consolidate_calendars" {
            guard verb == "fixture" else { throw TruthFailure.invalid }
            guard let proof = request["empty_calendar_proof"] as? [String: Any] else { throw TruthFailure.invalid }
            var result = try consolidate(store, proof: proof); result["status"] = "verified"
            return result
        }
        if action == "cleanup_empty_build_calendar" {
            guard verb == "fixture", request["earlier_build_cleanup_authorized"] as? Bool == true,
                  let proof = request["empty_calendar_proof"] as? [String: Any], proof["all_dates"] as? Bool == true, proof["read_only"] as? Bool == true,
                  let generated = proof["generated_at"] as? Double, (0...10).contains(Date().timeIntervalSince1970 - generated),
                  let counts = proof["counts"] as? [String: NSNumber] else { throw TruthFailure.invalid }
            var owned = try ownership()
            let calendars = store.calendars(for: .event).filter { $0.title == "Bridge Test" }
            guard calendars.allSatisfy({ owned.calendars[$0.calendarIdentifier] != nil && counts[$0.calendarIdentifier]?.intValue == 0 && $0.allowsContentModifications }) else { throw TruthFailure.invalid }
            for calendar in calendars {
                try store.removeCalendar(calendar, commit: true)
                owned.calendars.removeValue(forKey: calendar.calendarIdentifier)
                owned.calendarReceipts?.removeValue(forKey: calendar.calendarIdentifier)
                if owned.canonicalCalendar == calendar.calendarIdentifier { owned.canonicalCalendar = nil }
            }
            try saveOwnership(owned)
            return ["status": "verified", "removed_empty_build_calendars": calendars.count, "unbounded_item_count": 0]
        }
        let canonical = try ownership().canonicalCalendar
        var calendars = store.calendars(for: .event).filter { $0.title == "Bridge Test" }.sorted {
            if ($0.calendarIdentifier == canonical) != ($1.calendarIdentifier == canonical) { return $0.calendarIdentifier == canonical }
            return $0.calendarIdentifier < $1.calendarIdentifier
        }
        if let key = request["calendar_id"] as? String { calendars = calendars.filter { $0.calendarIdentifier == key } }
        if calendars.isEmpty && verb == "fixture" && ["create", "create_calendar"].contains(action) {
            guard let source = store.defaultCalendarForNewEvents?.source else { throw TruthFailure.unavailable }
            let calendar = EKCalendar(for: .event, eventStore: store)
            calendar.title = "Bridge Test"; calendar.source = source
            try store.saveCalendar(calendar, commit: true)
            var owned = try ownership(); owned.calendars[calendar.calendarIdentifier] = tag; owned.canonicalCalendar = calendar.calendarIdentifier; try saveOwnership(owned)
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
        for identifier in (verb == "fixture" || action == "audit" ? try ownership().events[tag] ?? [] : []) {
            if let event = store.event(withIdentifier: identifier), event.calendar.title == "Bridge Test", calendars.contains(where: { $0.calendarIdentifier == event.calendar.calendarIdentifier }) {
                let key = event.calendarItemIdentifier + "|" + String(event.occurrenceDate?.timeIntervalSince1970 ?? 0)
                if keys.insert(key).inserted { events.append(event) }
            }
        }
        let selected = events.filter { event in
            (event.title ?? "").hasPrefix(tag) && ((request["title"] as? String).map { event.title == $0 } ?? true) && ((request["occurrence_date"] as? String).map { stamp(event.occurrenceDate ?? event.startDate) == $0 } ?? true)
        }.sorted { $0.startDate < $1.startDate }
        if verb == "fixture" {
            switch action {
            case "create_calendar":
                guard let calendar = calendars.first else { throw TruthFailure.unavailable }
                return ["status": "verified", "id": calendar.calendarIdentifier]
            case "record_created_calendar":
                guard let key = request["calendar_id"] as? String, let calendar = calendars.first(where: { $0.calendarIdentifier == key }), calendar.title == "Bridge Test", let fingerprint = request["creation_fingerprint"] as? String, !fingerprint.isEmpty else { throw TruthFailure.invalid }
                var owned = try ownership(); owned.calendars[key] = tag
                var receipts = owned.calendarReceipts ?? [:]; receipts[key] = fingerprint; owned.calendarReceipts = receipts
                try saveOwnership(owned)
                return ["status": "verified", "ownership_recorded": true]
            case "create":
                guard let calendar = calendars.first, calendar.allowsContentModifications, let title = request["title"] as? String, title.hasPrefix(tag) else { throw TruthFailure.invalid }
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
                let effect = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
                let effectWall = Date().timeIntervalSince1970 * 1_000_000_000
                try remember(event, tag: tag)
                return ["status": "verified", "id": event.calendarItemIdentifier, "effect_uptime_ns": effect, "effect_wall_ns": effectWall]
            case "modify", "move":
                guard let index = request["index"] as? Int, selected.indices.contains(index) else { throw TruthFailure.invalid }
                let event = selected[index]
                guard event.calendar.title == "Bridge Test", event.calendar.allowsContentModifications else { throw TruthFailure.invalid }
                if let title = request["new_title"] as? String { guard title.hasPrefix(tag) else { throw TruthFailure.invalid }; event.title = title }
                if let value = request["start"] as? String {
                    let duration = event.endDate.timeIntervalSince(event.startDate)
                    event.startDate = try date(value, allDay: event.isAllDay, zone: event.isAllDay ? .current : (event.timeZone ?? .current))
                    event.endDate = event.startDate.addingTimeInterval(duration)
                }
                if let value = request["end"] as? String { event.endDate = try date(value, allDay: event.isAllDay, zone: event.isAllDay ? .current : (event.timeZone ?? .current)) }
                guard event.endDate > event.startDate else { throw TruthFailure.invalid }
                try store.save(event, span: request["span"] as? String == "future" ? .futureEvents : .thisEvent, commit: true)
                let effect = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
                let effectWall = Date().timeIntervalSince1970 * 1_000_000_000
                try remember(event, tag: tag)
                return ["status": "verified", "effect_uptime_ns": effect, "effect_wall_ns": effectWall]
            case "cleanup", "delete", "cleanup_calendar":
                if action == "cleanup_calendar" {
                    guard let key = request["calendar_id"] as? String, calendars.contains(where: { $0.calendarIdentifier == key && $0.title == "Bridge Test" }) else { throw TruthFailure.invalid }
                }
                for event in selected {
                    guard event.calendar.title == "Bridge Test", (event.title ?? "").hasPrefix(tag), event.calendar.allowsContentModifications else { throw TruthFailure.invalid }
                    try store.remove(event, span: .thisEvent, commit: true)
                }
                return ["status": "verified", "calendar_retained": true, "all_dates_ownership_proven": false, "removed_tagged_occurrences": selected.count]
            default: throw TruthFailure.invalid
            }
        }
        for event in selected { try remember(event, tag: tag) }
        let records: [[String: Any]] = selected.map { event in
            let zone = event.isAllDay ? TimeZone.current : (event.timeZone ?? .current)
            var row: [String: Any] = ["id": event.calendarItemIdentifier, "external_id": event.calendarItemExternalIdentifier ?? "", "calendar_id": event.calendar.calendarIdentifier, "title": event.title ?? "", "start": event.isAllDay ? floating(event.startDate, zone: zone) : stamp(event.startDate), "end": event.isAllDay ? floating(event.endDate, zone: zone) : stamp(event.endDate), "all_day": event.isAllDay, "time_zone": event.timeZone?.identifier ?? "", "is_exception": event.isDetached, "occurrence_date": event.occurrenceDate.map { event.isAllDay ? floating($0,zone:.current) : stamp($0) } ?? "", "recurring": event.hasRecurrenceRules]
            let external = event.calendarItemExternalIdentifier ?? event.calendarItemIdentifier
            row["series_id"] = event.isDetached ? external.components(separatedBy: "/RID=").first! : external
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
