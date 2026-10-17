import Foundation
import CryptoKit
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

@MainActor
public final class RemindersSource: Source {
    public nonisolated let name = "reminders"
    private let gate: AgendaGate
    private var committed: [String: JSON] = [:]
    private let commandsEnabled: Bool
    private let baselineURL: URL
    private var forceFull = true
    private var scanning = false
    public static let operations = ["reminders.create", "reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete", "reminders.create_list"]
    public static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("recurrence: next incomplete instance only")])])

    public init(gate: AgendaGate, home: URL, commandsEnabled: Bool = false) throws {
        self.gate = gate; self.commandsEnabled = commandsEnabled
        baselineURL = home.appendingPathComponent("reminders-baseline.json")
        if FileManager.default.fileExists(atPath: baselineURL.path) {
            committed = try JSONDecoder().decode([String: JSON].self, from: Data(contentsOf: baselineURL))
        }
    }

    private func save(_ baseline: [String: JSON]) throws {
        try JSONEncoder().encode(baseline).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
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

    private func hash(_ value: JSON) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return SHA256.hash(data: try encoder.encode(value)).map { String(format: "%02x", $0) }.joined()
    }

    private func observation(_ collection: String, _ key: String, _ value: JSON, alias: String = "") -> JSON {
        var result: [String: JSON] = ["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "data": value, "observed_at": .string(timestamp())]
        if !alias.isEmpty { result["scoped_alias"] = .string(alias) }
        return .object(result)
    }

    private func scan(into sink: any ObservationSink, live: Bool) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(10)) }
        scanning = true
        defer { scanning = false }
        let started = timestamp()
        let reminders = try await gate.reminders()
        let calendars = gate.store.calendars(for: .reminder)
        var current: [String: JSON] = [:]
        for calendar in calendars {
            let key = "reminders.list/" + calendar.calendarIdentifier
            current[key] = observation("reminders.list", calendar.calendarIdentifier, listData(calendar))
        }
        for reminder in reminders {
            let key = "reminders.reminder/" + reminder.calendarItemIdentifier
            let alias = reminder.calendarItemExternalIdentifier.map { reminder.calendar.calendarIdentifier + "/" + $0 } ?? ""
            current[key] = observation("reminders.reminder", reminder.calendarItemIdentifier, data(reminder), alias: alias)
        }
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        var observations: [JSON] = []
        for key in current.keys.sorted() {
            guard let observation = current[key] else { continue }
            let changed = try hash(observation["data"]) != hash(committed[key]?["data"] ?? .null)
            if forceFull || changed || committed[key]?["scoped_alias"] != observation["scoped_alias"] { observations.append(observation) }
        }
        for (key, previous) in committed where current[key] == nil {
            let alias = previous["scoped_alias"].string
            if !alias.isEmpty && current.values.contains(where: { $0["collection"] == previous["collection"] && $0["scoped_alias"].string == alias }) { continue }
            if previous["collection"].string == "reminders.reminder", gate.store.calendarItem(withIdentifier: previous["native_key"].string) != nil { continue }
            var absent = previous.object
            absent["kind"] = .string("absent"); absent["revalidated"] = .bool(true); absent["observed_at"] = .string(timestamp())
            observations.append(.object(absent))
        }
        try save(committed.merging(current) { _, new in new })
        try await sink.emit(source: name, generation: "eventkit-v1", observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["reminders.list", "reminders.reminder"], completionEvidence: "complete", blobHandles: [:], live: live)
        try save(current)
        committed = current
        forceFull = false
        let readiness = await open()
        try await sink.sourceState(source: name, state: readiness.state, coverage: readiness.coverage, capabilities: readiness.capabilities, reason: readiness.reason)
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
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(900)); self.gate.reconcile() } catch { return }
            }
        }
        defer { periodic.cancel() }
        for await _ in changes {
            try Task.checkCancellation()
            try await scan(into: sink, live: true)
        }
    }

    private func fresh(_ key: String) async throws -> EKReminder {
        guard let reminder = try await gate.reminders().first(where: { $0.calendarItemIdentifier == key }) else { throw SourceFailure.conflict }
        return reminder
    }

    public func prepare(_ command: JSON) async throws -> Prepared {
        guard commandsEnabled else { throw SourceFailure.unsupported }
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let action = command["action"]
        guard Self.operations.contains(action["op"].string) else { throw SourceFailure.unsupported }
        var before = JSON.null
        if !action["native_key"].string.isEmpty {
            let reminder = try await fresh(action["native_key"].string)
            before = data(reminder)
            if !action["expected_list_key"].string.isEmpty && reminder.calendar.calendarIdentifier != action["expected_list_key"].string { throw SourceFailure.conflict }
            var expected = command["expected_state"].object
            var native = before.object
            expected.removeValue(forKey: "list_id"); native.removeValue(forKey: "list_id")
            guard expected == native else { throw SourceFailure.conflict }
            guard reminder.calendar.allowsContentModifications else { throw SourceFailure.unsupported }
        }
        if action["op"].string != "reminders.create_list" {
            guard let calendar = gate.store.calendar(withIdentifier: action["list_key"].string), calendar.allowsContentModifications else { throw SourceFailure.unsupported }
        }
        if action["op"].string == "reminders.create_list" {
            guard gate.store.defaultCalendarForNewReminders()?.allowsContentModifications == true else { throw SourceFailure.unsupported }
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let encoded = try encoder.encode(action)
        return Prepared(nativeState: before, fingerprint: SHA256.hash(data: encoded).map { String(format: "%02x", $0) }.joined(), action: action)
    }

    private func patch(_ reminder: EKReminder, _ args: JSON) throws {
        if let title = args.object["title"] { reminder.title = title.string }
        if let notes = args.object["notes"] { reminder.notes = notes == .null ? nil : notes.string }
        if let url = args.object["url"] { reminder.url = url == .null ? nil : URL(string: url.string) }
        if let priority = args.object["priority"] {
            guard let value = Int(exactly: priority.number), (0...9).contains(value) else { throw SourceFailure.invalid }
            reminder.priority = value
        }
        for field in ["due", "start"] {
            guard let value = args.object[field] else { continue }
            var result: DateComponents?
            if value != .null {
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

    public func validate(_ prepared: Prepared) async throws {
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let action = prepared.action
        if action["op"].string != "reminders.create_list" {
            guard let calendar = gate.store.calendar(withIdentifier: action["list_key"].string), calendar.allowsContentModifications else { throw SourceFailure.unsupported }
        }
        if !action["native_key"].string.isEmpty {
            let current = try await fresh(action["native_key"].string)
            guard data(current) == prepared.nativeState else { throw SourceFailure.conflict }
            guard current.calendar.allowsContentModifications else { throw SourceFailure.unsupported }
        }
    }

    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        guard gate.authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let action = prepared.action
        let operation = action["op"].string
        var nativeKey = action["native_key"].string
        if operation == "reminders.create_list" {
            guard let source = gate.store.defaultCalendarForNewReminders()?.source else { throw SourceFailure.unavailable }
            let calendar = EKCalendar(for: .reminder, eventStore: gate.store)
            calendar.title = action["args"]["title"].string; calendar.source = source
            try gate.store.saveCalendar(calendar, commit: true)
            nativeKey = calendar.calendarIdentifier
            guard gate.store.calendar(withIdentifier: nativeKey)?.title == calendar.title else { throw SourceFailure.unavailable }
        } else {
            let reminder: EKReminder
            if operation == "reminders.create" {
                reminder = EKReminder(eventStore: gate.store)
                guard let calendar = gate.store.calendar(withIdentifier: action["list_key"].string), calendar.allowsContentModifications else { throw SourceFailure.unsupported }
                reminder.calendar = calendar
            } else {
                reminder = try await fresh(nativeKey)
                guard data(reminder) == prepared.nativeState else { throw SourceFailure.conflict }
                guard reminder.calendar.allowsContentModifications else { throw SourceFailure.unsupported }
            }
            if operation == "reminders.delete" { try gate.store.remove(reminder, commit: true) }
            else {
                if let destination = gate.store.calendar(withIdentifier: action["list_key"].string) { reminder.calendar = destination }
                try patch(reminder, action["args"])
                if operation == "reminders.complete" { reminder.isCompleted = true }
                if operation == "reminders.uncomplete" { reminder.isCompleted = false }
                try gate.store.save(reminder, commit: true)
                nativeKey = reminder.calendarItemIdentifier
            }
            let observed = try await gate.reminders().first(where: { $0.calendarItemIdentifier == nativeKey })
            if operation == "reminders.delete" {
                guard observed == nil else { throw SourceFailure.unavailable }
            } else {
                guard let observed, data(observed) == data(reminder) else { throw SourceFailure.unavailable }
            }
        }
        return .object(["native_key": .string(nativeKey), "fingerprint": .string(prepared.fingerprint), "observed": .bool(true), "absent": .bool(operation == "reminders.delete")])
    }

    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        let action = evidence["prepared"]["action"]
        let key = action["native_key"].string.isEmpty ? evidence["native_key"].string : action["native_key"].string
        if action["op"].string == "reminders.create_list" {
            guard gate.authorized(.reminder), !key.isEmpty, let list = gate.store.calendar(withIdentifier: key), list.title == action["args"]["title"].string else { return .null }
            return .object(["native_key": .string(key), "observed": .bool(true), "prepared": evidence["prepared"]])
        }
        guard !key.isEmpty, let reminders = try? await gate.reminders() else { return .null }
        let current = reminders.first { $0.calendarItemIdentifier == key }
        let operation = action["op"].string
        if operation == "reminders.delete" {
            guard current == nil else { return .null }
        } else {
            guard let current else { return .null }
            let actual = data(current)
            if operation == "reminders.complete" && !current.isCompleted { return .null }
            if operation == "reminders.uncomplete" && current.isCompleted { return .null }
            if operation == "reminders.update" || operation == "reminders.create" {
                let fields = action["args"].object.filter { !["id", "list_id", "due_all_day"].contains($0.key) }
                guard !fields.isEmpty, fields.allSatisfy({ actual[$0.key] == $0.value }) else { return .null }
            }
        }
        return .object(["native_key": .string(key), "observed": .bool(true), "absent": .bool(current == nil), "resolved": .bool(true), "prepared": evidence["prepared"]])
    }
}
