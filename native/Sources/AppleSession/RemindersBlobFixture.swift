#if LAB
import Foundation
import EventKit
import RemindersStore
import SourceSupport

@MainActor
enum RemindersBlobFixture {
    private struct Fetched: @unchecked Sendable { let reminders: [EKReminder]? }
    private final class FetchCompletion: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<Fetched, Never>?
        init(_ continuation: CheckedContinuation<Fetched, Never>) { self.continuation = continuation }
        func finish(_ value: Fetched) {
            lock.lock()
            let pending = continuation
            continuation = nil
            lock.unlock()
            pending?.resume(returning: value)
        }
    }

    static func run(journal: URL) async throws -> JSON {
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { throw SourceFailure.unavailable }
        let store = EKEventStore()
        guard let source = store.defaultCalendarForNewReminders()?.source else { throw SourceFailure.unavailable }
        let tag = "[ab-test store-decode-" + UUID().uuidString + "]"
        let list = EKCalendar(for: .reminder, eventStore: store)
        list.title = "Bridge Test " + tag
        list.source = source
        try store.saveCalendar(list, commit: true)
        let listID = list.calendarIdentifier
        var result: JSON
        do {
            try JSONEncoder().encode(JSON.object(["fixture_tag": .string(tag), "list_id": .string(listID), "created": .bool(true)])).write(to: journal, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: journal.path)
            let components = DateComponents(calendar: Calendar(identifier: .gregorian), timeZone: TimeZone(secondsFromGMT: 0), year: 2040, month: 1, day: 2, hour: 14, minute: 0)
            guard let due = components.date else { throw SourceFailure.invalid }
            var identifiers: [String: String] = [:]
            for (label, offset) in [("control", nil), ("absolute_15m", -900.0), ("absolute_1d", -86400.0), ("relative_15m", -900.0), ("relative_1d", -86400.0)] as [(String, Double?)] {
                let reminder = EKReminder(eventStore: store)
                reminder.calendar = list
                reminder.title = tag + " " + label
                reminder.startDateComponents = components
                reminder.dueDateComponents = components
                if let offset { reminder.addAlarm(label.hasPrefix("relative_") ? EKAlarm(relativeOffset: offset) : EKAlarm(absoluteDate: due.addingTimeInterval(offset))) }
                try store.save(reminder, commit: true)
                identifiers[reminder.calendarItemIdentifier] = label
            }
            result = try await read(identifiers: identifiers, listID: listID)
        } catch {
            try await cleanup(listID: listID, tag: tag)
            throw error
        }
        try await cleanup(listID: listID, tag: tag)
        var fields = result.object
        fields["created_fixture_cleaned_and_swept"] = .bool(true)
        return .object(fields)
    }

    private static func read(identifiers: [String: String], listID: String) async throws -> JSON {
        let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        var captured: JSON?
        repeat {
            var reminders: [String: JSON] = [:]
            var lists: [JSON] = []
            for session in sessions {
                let snapshot = try await session.read().snapshot
                for reminder in snapshot.reminders.values {
                    guard let identifier = reminder.calendarItemIdentifier, identifier == reminder.identity.uuid?.uuidString,
                          let label = identifiers[identifier] else { continue }
                    reminders[label] = .object(["fixture_identifier": .string(identifier), "early_reminder": blob(reminder.earlyReminder)])
                }
                for list in snapshot.lists.values where list.identity.uuid?.uuidString == listID {
                    lists.append(.object(["manual_order_exact": .bool(manualOrderExact(list.orderingJSON, identifiers: identifiers)), "fixture_identifier": .string(listID), "section_order": blob(list.sectionOrder), "section_membership": blob(list.sectionMembership), "smart_filter": blob(list.filter), "ordering": blob(list.ordering), "ordering_json": blob(list.orderingJSON)]))
                }
            }
            if reminders.count == identifiers.count, lists.count == 1 {
                captured = .object(["fixture": .string("EventKit absolute and relative alarms on a timed due date"), "reminders": .object(reminders), "list": lists[0]])
                if reminders.values.contains(where: { $0["early_reminder"] != .null }), lists[0]["manual_order_exact"].bool, let captured { return captured }
            }
            try await Task.sleep(for: .milliseconds(20))
        } while ContinuousClock.now < deadline
        if let captured { return captured }
        throw SourceFailure.unavailable
    }

    private static func manualOrderExact(_ data: Data?, identifiers: [String: String]) -> Bool {
        guard case .supported(let positions) = StoreManualOrder.decode(data), positions.count == identifiers.count else { return false }
        let labels = ["control", "absolute_15m", "absolute_1d", "relative_15m", "relative_1d"]
        return identifiers.allSatisfy { identifier, label in
            guard let uuid = UUID(uuidString: identifier), let index = labels.firstIndex(of: label) else { return false }
            return positions[uuid] == index
        }
    }

    private static func blob(_ data: Data?) -> JSON {
        guard let data else { return .null }
        return .object(["byte_count": .number(Double(data.count)), "base64": .string(data.base64EncodedString())])
    }

    static func recover(journal: URL) async throws -> JSON {
        let registration = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: journal))
        if registration["created_fixture_cleaned_and_swept"].bool { return registration }
        let tag = registration["fixture_tag"].string
        let listID = registration["list_id"].string
        guard registration["created"].bool, tag.hasPrefix("[ab-test store-decode-"), tag.hasSuffix("]"), !listID.isEmpty else { throw SourceFailure.invalid }
        try await cleanup(listID: listID, tag: tag)
        return .object(["created_fixture_cleaned_and_swept": .bool(true), "recovered": .bool(true)])
    }

    private static func cleanup(listID: String, tag: String) async throws {
        let store = EKEventStore()
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { throw SourceFailure.unavailable }
        guard let list = store.calendar(withIdentifier: listID) else {
            guard !store.calendars(for: .reminder).contains(where: { $0.title == "Bridge Test " + tag }) else { throw SourceFailure.conflict }
            return
        }
        guard list.title == "Bridge Test " + tag else { throw SourceFailure.conflict }
        let predicate = store.predicateForReminders(in: [list])
        let fetched: Fetched = await withCheckedContinuation { continuation in
            let completion = FetchCompletion(continuation)
            store.fetchReminders(matching: predicate) { completion.finish(Fetched(reminders: $0)) }
            Task {
                try? await Task.sleep(for: .seconds(8))
                completion.finish(Fetched(reminders: nil))
            }
        }
        guard let all = fetched.reminders else { throw SourceFailure.unavailable }
        guard all.allSatisfy({ ($0.title ?? "").hasPrefix(tag + " ") }) else { throw SourceFailure.conflict }
        try store.removeCalendar(list, commit: true)
        let sweep = EKEventStore()
        guard !sweep.calendars(for: .reminder).contains(where: { $0.calendarIdentifier == listID || $0.title == "Bridge Test " + tag }) else { throw SourceFailure.conflict }
    }
}
#endif
