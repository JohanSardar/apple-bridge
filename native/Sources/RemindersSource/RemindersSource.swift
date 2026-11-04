import Foundation
import CryptoKit
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@AgendaActor
public final class RemindersSource: Source {
    public nonisolated let name = "reminders"
    private let gate: AgendaGate
    private let commandsEnabled: Bool
    public nonisolated static let operations = ["reminders.create", "reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete", "reminders.create_list", "reminders.rename_list", "reminders.delete_list"]
    public nonisolated static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("recurrence: next incomplete instance only")]), "operations": RemindersParity.coverage])

    public nonisolated init(gate: AgendaGate, commandsEnabled: Bool = false) {
        self.gate = gate; self.commandsEnabled = commandsEnabled
    }

    private func lineageGeneration() throws -> String {
        let identity = gate.store.eventStoreIdentifier
        guard !identity.isEmpty else { throw SourceFailure.unavailable }
        return "eventkit-" + SHA256.hash(data: Data(identity.utf8)).map { String(format: "%02x", $0) }.joined()
    }





    public func open() async -> SourceReadiness {
        SourceReadiness(state: gate.authorized(.reminder) ? "ready" : "permission_missing", coverage: Self.coverage, capabilities: commandsEnabled && gate.authorized(.reminder) ? Self.operations : [], reason: gate.authorized(.reminder) ? "" : "reminders_full_access_required")
    }

    private func listData(_ calendar: EKCalendar) -> JSON {
        .object(["title": .string(calendar.title), "account": .string(calendar.source.sourceIdentifier), "color": .string(calendar.cgColor.map { String(describing: $0.components ?? []) } ?? ""), "writable": .bool(calendar.allowsContentModifications)])
    }

    private func components(_ value: DateComponents?) -> JSON {
        guard let value else { return .null }
        if value.hour == nil { return .string(String(format: "%04d-%02d-%02d", value.year ?? 0, value.month ?? 0, value.day ?? 0)) }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = value.timeZone ?? .current
        return calendar.date(from: value).map { .string(timestamp($0)) } ?? .null
    }

    private func recurrence(_ rules: [EKRecurrenceRule]?) -> JSON {
        guard let rules, !rules.isEmpty else { return .null }
        func numbers(_ values: [NSNumber]?) -> JSON { .array((values ?? []).map { .number($0.doubleValue) }) }
        return .array(rules.map { rule in
            .object(["frequency": .string(["daily", "weekly", "monthly", "yearly"][Int(rule.frequency.rawValue)]), "interval": .number(Double(rule.interval)), "end": rule.recurrenceEnd?.endDate.map { .string(timestamp($0)) } ?? .null, "count": rule.recurrenceEnd.map { .number(Double($0.occurrenceCount)) } ?? .null, "first_day_of_week": .number(Double(rule.firstDayOfTheWeek)), "days_of_week": .array((rule.daysOfTheWeek ?? []).map { .object(["day": .number(Double($0.dayOfTheWeek.rawValue)), "week": .number(Double($0.weekNumber))]) }), "days_of_month": numbers(rule.daysOfTheMonth), "days_of_year": numbers(rule.daysOfTheYear), "weeks_of_year": numbers(rule.weeksOfTheYear), "months": numbers(rule.monthsOfTheYear), "positions": numbers(rule.setPositions)])
        })
    }

    private func alarms(_ alarms: [EKAlarm]?) -> JSON {
        .array((alarms ?? []).map { alarm in
            var fields: [String: JSON] = ["absolute_date": alarm.absoluteDate.map { .string(timestamp($0)) } ?? .null, "relative_offset": .number(alarm.relativeOffset), "proximity": .number(Double(alarm.proximity.rawValue))]
            if let location = alarm.structuredLocation {
                var place: [String: JSON] = ["title": location.title.map(JSON.string) ?? .null, "radius": .number(location.radius)]
                if let coordinate = location.geoLocation?.coordinate { place["latitude"] = .number(coordinate.latitude); place["longitude"] = .number(coordinate.longitude) }
                fields["location"] = .object(place)
            }
            return .object(fields)
        })
    }

    private func data(_ reminder: EKReminder) -> JSON {
        .object(["title": .string(reminder.title ?? ""), "notes": reminder.notes.map(JSON.string) ?? .null, "url": reminder.url.map { .string($0.absoluteString) } ?? .null, "due": components(reminder.dueDateComponents), "due_all_day": .bool(reminder.dueDateComponents != nil && reminder.dueDateComponents?.hour == nil), "start": components(reminder.startDateComponents), "priority": .number(Double(reminder.priority)), "completed": .bool(reminder.isCompleted), "completed_at": reminder.completionDate.map { .string(timestamp($0)) } ?? .null, "recurrence": recurrence(reminder.recurrenceRules), "alarms": alarms(reminder.alarms), "list_id": .object(["$ref": .object(["collection": .string("reminders.list"), "native_key": .string(reminder.calendar.calendarIdentifier)])])])
    }

    private func observation(_ collection: String, _ key: String, _ value: JSON, alias: String = "", at: String? = nil) -> JSON {
        var result: [String: JSON] = ["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "data": value, "observed_at": .string(at ?? timestamp())]
        if !alias.isEmpty { result["scoped_alias"] = .string(alias) }
        return .object(result)
    }




    public func enumerate() async throws -> SourceEnumeration {
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        let generation = try lineageGeneration()
        let calendars = gate.store.calendars(for: .reminder)
        let reminders = try await gate.reminders()
        let observedAt = timestamp()
        var current: [String: JSON] = [:]
        for calendar in calendars {
            current["reminders.list/" + calendar.calendarIdentifier] = observation("reminders.list", calendar.calendarIdentifier, listData(calendar), at: observedAt)
        }
        for reminder in reminders {
            let alias = reminder.calendarItemExternalIdentifier.map { reminder.calendar.calendarIdentifier + "/" + $0 } ?? ""
            current["reminders.reminder/" + reminder.calendarItemIdentifier] = observation("reminders.reminder", reminder.calendarItemIdentifier, data(reminder), alias: alias, at: observedAt)
        }
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        return SourceEnumeration(generation: generation, rows: current.keys.sorted().compactMap { current[$0] })
    }

    public func revalidate(_ identities: [JSON]) async throws -> [String] {
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        var states: [String] = []
        for identity in identities {
            let key = identity["native_key"].string
            switch identity["collection"].string {
            case "reminders.list":
                states.append(gate.store.calendar(withIdentifier: key) == nil ? RevalidationState.missing : RevalidationState.present)
            case "reminders.reminder":
                states.append(gate.store.calendarItem(withIdentifier: key) is EKReminder ? RevalidationState.present : RevalidationState.missing)
            default: throw SourceFailure.invalid
            }
            guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        }
        return states
    }



    private func fresh(_ key: String) async throws -> EKReminder {
        guard let reminder = try await gate.reminders().first(where: { $0.calendarItemIdentifier == key }) else { throw SourceFailure.conflict }
        return reminder
    }


    private func patch(_ reminder: EKReminder, _ args: JSON) throws {
        if let title = args.object["title"] { reminder.title = title.string }
        if let notes = args.object["notes"] { reminder.notes = notes == .null || notes.string.isEmpty ? nil : notes.string }
        if let url = args.object["url"] { reminder.url = url == .null || url.string.isEmpty ? nil : URL(string: url.string) }
        if let priority = args.object["priority"] {
            guard let value = Int(exactly: priority.number), (0...9).contains(value) else { throw SourceFailure.invalid }
            reminder.priority = value
        }
        for field in ["due", "start"] {
            guard let value = args.object[field] else { continue }
            var result: DateComponents?
            if value != .null && !value.string.isEmpty {
                if let date = parseTimestamp(value.string) {
                    let fields: Set<Calendar.Component> = args["due_all_day"].bool ? [.year, .month, .day] : [.year, .month, .day, .hour, .minute, .second, .timeZone]
                    result = Calendar.current.dateComponents(fields, from: date)
                } else {
                    let parts = value.string.split(separator: "-").compactMap { Int($0) }
                    guard parts.count == 3, (1...12).contains(parts[1]), (1...31).contains(parts[2]) else { throw SourceFailure.invalid }
                    result = DateComponents(year: parts[0], month: parts[1], day: parts[2])
                }
            }
            if field == "due" { reminder.dueDateComponents = result } else { reminder.startDateComponents = result }
        }
    }




    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard commandsEnabled else { throw SourceFailure.unsupported }
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let key = args["native_key"].string
        switch operation {
        case "reminders.read_list":
            guard !key.isEmpty, let list = gate.store.calendar(withIdentifier: key) else { return .object(["exists": .bool(false)]) }
            return .object(["exists": .bool(true), "writable": .bool(list.allowsContentModifications), "data": listData(list)])
        case "reminders.read_reminder":
            guard !key.isEmpty, let reminder = try await gate.reminders().first(where: { $0.calendarItemIdentifier == key }) else { return .object(["exists": .bool(false)]) }
            return .object(["exists": .bool(true), "writable": .bool(reminder.calendar.allowsContentModifications), "list_key": .string(reminder.calendar.calendarIdentifier), "data": data(reminder)])
        case "reminders.can_create_list":
            return .object(["allowed": .bool(gate.store.defaultCalendarForNewReminders()?.allowsContentModifications == true)])
        case "reminders.write":
            return try await writeNative(args)
        default:
            throw SourceFailure.unsupported
        }
    }

    private func writeNative(_ args: JSON) async throws -> JSON {
        let operation = args["op"].string
        guard Self.operations.contains(operation) else { throw SourceFailure.unsupported }
        var key = args["native_key"].string
        var saved = JSON.null
        var observed = JSON.null
        var existsAfter = false
        if operation == "reminders.rename_list" || operation == "reminders.delete_list" {
            guard let list = gate.store.calendar(withIdentifier: key) else { throw SourceFailure.unavailable }
            if operation == "reminders.delete_list" { try gate.store.removeCalendar(list, commit: true) }
            else { list.title = args["args"]["title"].string; try gate.store.saveCalendar(list, commit: true); key = list.calendarIdentifier; saved = listData(list) }
            let after = gate.store.calendars(for: .reminder).first { $0.calendarIdentifier == key }
            existsAfter = after != nil
            observed = after.map(listData) ?? .null
        } else if operation == "reminders.create_list" {
            guard let source = gate.store.defaultCalendarForNewReminders()?.source else { throw SourceFailure.unavailable }
            let calendar = EKCalendar(for: .reminder, eventStore: gate.store)
            calendar.title = args["args"]["title"].string; calendar.source = source
            try gate.store.saveCalendar(calendar, commit: true)
            key = calendar.calendarIdentifier
            saved = listData(calendar)
            let after = gate.store.calendar(withIdentifier: key)
            existsAfter = after != nil
            observed = after.map(listData) ?? .null
        } else {
            let reminder: EKReminder
            if operation == "reminders.create" {
                reminder = EKReminder(eventStore: gate.store)
                guard let calendar = gate.store.calendar(withIdentifier: args["list_key"].string) else { throw SourceFailure.unavailable }
                reminder.calendar = calendar
            } else {
                reminder = try await fresh(key)
            }
            if operation == "reminders.delete" { try gate.store.remove(reminder, commit: true) }
            else {
                if let destination = gate.store.calendar(withIdentifier: args["list_key"].string) { reminder.calendar = destination }
                try patch(reminder, args["args"])
                if operation == "reminders.complete" { reminder.isCompleted = true }
                if operation == "reminders.uncomplete" { reminder.isCompleted = false }
                try gate.store.save(reminder, commit: true)
                key = reminder.calendarItemIdentifier
                saved = data(reminder)
            }
            let after = try await gate.reminders().first(where: { $0.calendarItemIdentifier == key })
            existsAfter = after != nil
            observed = after.map(data) ?? .null
        }
        return .object(["native_key": .string(key), "saved": saved, "observed": observed, "exists_after": .bool(existsAfter)])
    }
}
