import Foundation
@preconcurrency import EventKit

@MainActor
struct CalendarMetadataProbe {
    static let fields = ["calendar_id", "title", "notes", "location", "url", "start", "end", "all_day", "time_zone", "is_exception", "availability", "status", "series_id", "occurrence_date", "attendee_name", "attendee_url", "attendee_status", "attendee_role", "attendee_type", "alarm_absolute", "alarm_relative", "recurrence_frequency", "recurrence_interval", "recurrence_count", "recurrence_end", "recurrence_first_day", "recurrence_weekday", "recurrence_monthday", "recurrence_yearday", "recurrence_yearweek", "recurrence_month", "recurrence_position"]

    static func run(_ request: [String: Any]) throws -> [String: Any] {
        guard let tag = request["tag"] as? String, let identifier = request["calendar_id"] as? String else { throw TruthFailure.invalid }
        let store = EKEventStore()
        guard let calendar = store.calendar(withIdentifier: identifier), calendar.title == "Bridge Test " + tag, calendar.allowsContentModifications else { throw TruthFailure.invalid }
        let other = EKCalendar(for: .event, eventStore: store)
        other.title = calendar.title
        other.source = calendar.source
        try store.saveCalendar(other, commit: true)
        let otherIdentifier = other.calendarIdentifier
        _ = try CalendarTruth.run("fixture", ["action": "record_created_calendar", "tag": tag, "calendar_id": other.calendarIdentifier, "creation_fingerprint": "metadata_probe_created"])
        let start = Date(timeIntervalSince1970: 1_793_520_000)
        let unsupported: [String: String] = ["status": "read_only_public_api", "series_id": "derived_identity", "occurrence_date": "read_only_original_occurrence", "recurrence_first_day": "read_only_public_api", "attendee_name": "no_public_participant_setter", "attendee_url": "no_public_participant_setter", "attendee_status": "remote_sync_not_exercised", "attendee_role": "no_public_participant_setter", "attendee_type": "no_public_participant_setter"]
        var fixtures: [String: String] = [:]
        for (index, field) in fields.enumerated() where unsupported[field] == nil {
            let event = EKEvent(eventStore: store)
            event.calendar = calendar
            let fixtureTitle = tag + " metadata " + String(index)
            event.title = fixtureTitle
            event.startDate = start
            event.endDate = start.addingTimeInterval(3600)
            event.timeZone = TimeZone(identifier: "America/New_York")
            event.availability = .busy
            event.alarms = [EKAlarm(relativeOffset: -300)]
            if field.hasPrefix("recurrence_") {
                event.addRecurrenceRule(try recurrence(field, start: start, edited: false))
            } else if field == "is_exception" {
                event.addRecurrenceRule(EKRecurrenceRule(recurrenceWith: .daily, interval: 1, end: EKRecurrenceEnd(occurrenceCount: 3)))
            }
            try store.save(event, span: .thisEvent, commit: true)
            guard event.eventIdentifier != nil else { throw TruthFailure.unavailable }
            fixtures[field] = fixtureTitle
        }
        Thread.sleep(forTimeInterval: 2)
        var rows: [[String: Any]] = []
        for field in fields {
            if let reason = unsupported[field] {
                rows.append(["field": field, "exercised": false, "reason": reason])
                continue
            }
            var stage = "fixture_lookup"
            do {
                store.reset()
                let reader = store
                guard let title = fixtures[field], var event = find(reader, title: title, tag: tag, start: start) else { throw TruthFailure.unavailable }
                if field == "is_exception" {
                    guard let own = reader.calendar(withIdentifier: identifier) else { throw TruthFailure.unavailable }
                    let day = start.addingTimeInterval(86400)
                    let baselineTitle = event.title
                    guard let occurrence = reader.events(matching: reader.predicateForEvents(withStart: day, end: day.addingTimeInterval(3600), calendars: [own])).first(where: { $0.title == baselineTitle }) else { throw TruthFailure.unavailable }
                    event = occurrence
                }
                stage = "pre_read"
                let beforeIdentifier = event.calendarItemIdentifier
                let beforeModified = event.lastModifiedDate
                let before = value(event, field)
                try edit(event, field, other: otherIdentifier, store: reader)
                stage = "native_save"
                try reader.save(event, span: field.hasPrefix("recurrence_") ? .futureEvents : .thisEvent, commit: true)
                stage = "post_read"
                let savedTitle = event.title ?? title
                let savedIdentifier = event.calendarItemIdentifier
                reader.reset()
                let afterStore = reader
                var saved = find(afterStore, title: savedTitle, tag: tag, start: start, preferred: savedIdentifier)
                if field == "is_exception", let own = afterStore.calendar(withIdentifier: identifier) {
                    let day = start.addingTimeInterval(86400)
                    saved = afterStore.events(matching: afterStore.predicateForEvents(withStart: day, end: day.addingTimeInterval(3600), calendars: [own])).first(where: { $0.title == savedTitle })
                }
                guard let saved else { throw TruthFailure.unavailable }
                let afterModified = saved.lastModifiedDate
                var row: [String: Any] = ["field": field, "exercised": true, "field_changed": before != value(saved, field), "before_modified_present": beforeModified != nil, "after_modified_present": afterModified != nil, "modified_changed": beforeModified != afterModified, "native_identity_changed": beforeIdentifier != saved.calendarItemIdentifier]
                if let beforeModified, let afterModified { row["modified_delta_seconds"] = afterModified.timeIntervalSince(beforeModified) }
                rows.append(row)
            } catch {
                let visibleCalendars = store.calendars(for: .event).filter { $0.title == "Bridge Test " + tag }
                let visibleEvents = visibleCalendars.isEmpty ? [] : store.events(matching: store.predicateForEvents(withStart: start.addingTimeInterval(-86400), end: start.addingTimeInterval(730 * 86400), calendars: visibleCalendars))
                rows.append(["field": field, "exercised": false, "reason": "native_probe_error", "stage": stage, "error_domain": (error as NSError).domain, "error_code": (error as NSError).code, "fixture_key_present": fixtures[field] != nil, "visible_own_calendars": visibleCalendars.count, "visible_tagged_occurrences": visibleEvents.filter { ($0.title ?? "").hasPrefix(tag) }.count])
            }
        }
        return ["status": "verified", "fields": rows, "remote_attendee_sync_exercised": false, "public_eventkit_saves_only": true, "fixtures_saved": fixtures.count, "probe_strategy": "single_store_reset_and_refetch"]
    }

    private static func find(_ store: EKEventStore, title: String, tag: String, start: Date, preferred: String? = nil) -> EKEvent? {
        let calendars = store.calendars(for: .event).filter { $0.title == "Bridge Test " + tag }
        guard !calendars.isEmpty else { return nil }
        let events = store.events(matching: store.predicateForEvents(withStart: start.addingTimeInterval(-86400), end: start.addingTimeInterval(730 * 86400), calendars: calendars)).filter { $0.title == title && $0.startDate != nil && $0.endDate != nil }
        if let preferred, let event = events.first(where: { $0.calendarItemIdentifier == preferred }) { return event }
        return events.min { $0.startDate < $1.startDate }
    }

    private static func value(_ event: EKEvent, _ field: String) -> String {
        switch field {
        case "calendar_id": return event.calendar.calendarIdentifier
        case "title": return event.title ?? ""
        case "notes": return event.notes ?? ""
        case "location": return event.location ?? ""
        case "url": return event.url?.absoluteString ?? ""
        case "start": return String(event.startDate.timeIntervalSince1970)
        case "end": return String(event.endDate.timeIntervalSince1970)
        case "all_day": return String(event.isAllDay)
        case "time_zone": return event.timeZone?.identifier ?? ""
        case "is_exception": return String(event.isDetached)
        case "availability": return String(event.availability.rawValue)
        case "alarm_absolute": return String(describing: event.alarms?.first?.absoluteDate)
        case "alarm_relative": return String(describing: event.alarms?.first?.relativeOffset)
        default:
            guard let rule = event.recurrenceRules?.first else { return "none" }
            switch field {
            case "recurrence_frequency": return String(rule.frequency.rawValue)
            case "recurrence_interval": return String(rule.interval)
            case "recurrence_count": return String(describing: rule.recurrenceEnd?.occurrenceCount)
            case "recurrence_end": return String(describing: rule.recurrenceEnd?.endDate)
            case "recurrence_weekday": return (rule.daysOfTheWeek ?? []).map { String($0.dayOfTheWeek.rawValue) + ":" + String($0.weekNumber) }.joined(separator: ",")
            case "recurrence_monthday": return String(describing: rule.daysOfTheMonth)
            case "recurrence_yearday": return String(describing: rule.daysOfTheYear)
            case "recurrence_yearweek": return String(describing: rule.weeksOfTheYear)
            case "recurrence_month": return String(describing: rule.monthsOfTheYear)
            case "recurrence_position": return String(describing: rule.setPositions)
            default: return "unsupported"
            }
        }
    }

    private static func edit(_ event: EKEvent, _ field: String, other: String, store: EKEventStore) throws {
        switch field {
        case "calendar_id": guard let calendar = store.calendar(withIdentifier: other) else { throw TruthFailure.unavailable }; event.calendar = calendar
        case "title", "is_exception": event.title = (event.title ?? "") + " edited"
        case "notes": event.notes = "Bridge Test metadata"
        case "location": event.location = "Bridge Test room"
        case "url": event.url = URL(string: "https://example.com/calendar-metadata")
        case "start": event.startDate = event.startDate.addingTimeInterval(60)
        case "end": event.endDate = event.endDate.addingTimeInterval(60)
        case "all_day": event.isAllDay = true
        case "time_zone": event.timeZone = TimeZone(identifier: "Europe/Paris")
        case "availability": event.availability = .free
        case "alarm_absolute": event.alarms = [EKAlarm(absoluteDate: event.startDate.addingTimeInterval(-600))]
        case "alarm_relative": event.alarms = [EKAlarm(relativeOffset: -600)]
        default:
            event.recurrenceRules = [try recurrence(field, start: event.startDate, edited: true)]
        }
    }

    private static func recurrence(_ field: String, start: Date, edited: Bool) throws -> EKRecurrenceRule {
        var frequency: EKRecurrenceFrequency = .daily
        var interval = 1
        var end = EKRecurrenceEnd(occurrenceCount: 3)
        var weekdays: [EKRecurrenceDayOfWeek]?
        var monthdays: [NSNumber]?
        var yeardays: [NSNumber]?
        var yearweeks: [NSNumber]?
        var months: [NSNumber]?
        var positions: [NSNumber]?
        switch field {
        case "recurrence_frequency": frequency = edited ? .weekly : .daily
        case "recurrence_interval": interval = edited ? 2 : 1
        case "recurrence_count": end = EKRecurrenceEnd(occurrenceCount: edited ? 4 : 3)
        case "recurrence_end": if edited { end = EKRecurrenceEnd(end: start.addingTimeInterval(10 * 86400)) }
        case "recurrence_weekday": frequency = .weekly; weekdays = [EKRecurrenceDayOfWeek(edited ? .monday : .tuesday)]
        case "recurrence_monthday": frequency = .monthly; monthdays = [NSNumber(value: edited ? 2 : 1)]
        case "recurrence_yearday": frequency = .yearly; yeardays = [NSNumber(value: edited ? 3 : 2)]
        case "recurrence_yearweek": frequency = .yearly; yearweeks = [NSNumber(value: edited ? 4 : 3)]
        case "recurrence_month": frequency = .yearly; months = [NSNumber(value: edited ? 5 : 4)]
        case "recurrence_position": frequency = .monthly; weekdays = [EKWeekday.monday, .tuesday, .wednesday, .thursday, .friday].map { EKRecurrenceDayOfWeek($0) }; positions = [NSNumber(value: edited ? -1 : -2)]
        default: throw TruthFailure.invalid
        }
        return EKRecurrenceRule(recurrenceWith: frequency, interval: interval, daysOfTheWeek: weekdays, daysOfTheMonth: monthdays, monthsOfTheYear: months, weeksOfTheYear: yearweeks, daysOfTheYear: yeardays, setPositions: positions, end: end)
    }
}
