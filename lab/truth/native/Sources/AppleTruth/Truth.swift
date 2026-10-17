import Foundation
@preconcurrency import EventKit
import SQLite3
import Contacts
import Darwin
import CryptoKit

private struct Fetch: @unchecked Sendable { let reminders: [EKReminder]? }
private enum TruthFailure: Error { case invalid, unavailable, permissionMissing }

@main
struct AppleTruth {
    @MainActor
    static func main() {
        let arguments = Array(CommandLine.arguments.dropFirst())
        func option(_ name: String) -> String? {
            guard let index = arguments.firstIndex(of: name), index + 1 < arguments.count else { return nil }
            return arguments[index + 1]
        }
        guard let reportPath = option("--report"), let verb = arguments.first else { exit(2) }
        let deadline = DispatchSource.makeTimerSource(queue: .global())
        deadline.schedule(deadline: .now() + (verb == "authorize" ? 300 : 90))
        deadline.setEventHandler { @Sendable in
            let url = URL(fileURLWithPath: reportPath)
            try? Data("{\"status\":\"unverified\",\"reason\":\"deadline_exceeded\"}".utf8).write(to: url, options: .atomic)
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportPath)
            exit(1)
        }
        deadline.resume()
        defer { deadline.cancel() }
        var finished = false
        Task { @MainActor in
            var response: [String: Any]
            do {
                if verb == "authorize" {
                    let store = EKEventStore()
                    let granted: Bool
                    switch option("--source") {
                    case "reminders": granted = try await store.requestFullAccessToReminders()
                    case "calendar": granted = try await store.requestFullAccessToEvents()
                    case "contacts": granted = try await CNContactStore().requestAccess(for: .contacts)
                    default: throw TruthFailure.invalid
                    }
                    response = ["status": granted ? "verified" : "unverified", "granted": granted]
                } else {
                guard let requestPath = option("--request-file"), let request = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: requestPath))) as? [String: Any] else { throw TruthFailure.invalid }
                if request["source"] as? String == "messages" {
                    guard verb == "observe" else { throw TruthFailure.invalid }
                    response = try messages(request)
                } else { response = try await reminders(verb, request) }
                }
            } catch {
                response = ["status": "unverified", "reason": String(describing: error), "error_domain": (error as NSError).domain, "error_code": (error as NSError).code]
            }
            do {
                let url = URL(fileURLWithPath: reportPath)
                try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let bytes = try JSONSerialization.data(withJSONObject: response, options: [.sortedKeys])
                try bytes.write(to: url, options: .atomic)
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportPath)
            } catch { exit(3) }
            finished = true
        }
        while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
    }

    @MainActor
    private static func reminders(_ verb: String, _ request: [String: Any]) async throws -> [String: Any] {
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { throw TruthFailure.permissionMissing }
        let store = EKEventStore()
        let listName = request["list"] as? String ?? "Bridge Test"
        let action = request["action"] as? String ?? ""
        let tag = request["tag"] as? String ?? ""
        let title = request["title"] as? String ?? ""
        if verb == "fixture" {
            guard listName == "Bridge Test", tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), tag.count > 11, !tag.contains("\n") else { throw TruthFailure.invalid }
        } else if verb != "observe" { throw TruthFailure.invalid }
        var lists = store.calendars(for: .reminder).filter { $0.title == listName }
        if let listID = request["list_id"] as? String { lists = lists.filter { $0.calendarIdentifier == listID } }
        if lists.count > 1 {
            guard verb == "observe" || action == "cleanup" else { throw TruthFailure.invalid }
            var records: [[String: Any]] = []
            for list in lists {
                var scoped = request
                scoped["list_id"] = list.calendarIdentifier
                let result = try await reminders(verb, scoped)
                records += result["records"] as? [[String: Any]] ?? []
            }
            return ["status": "verified", "list_exists": true, "records": records, "lists": lists.map { ["id": $0.calendarIdentifier, "title": $0.title] }]
        }
        if lists.isEmpty && verb == "fixture" && (action == "create" || action == "create_list" || action == "burst") {
            guard let source = store.defaultCalendarForNewReminders()?.source else { throw TruthFailure.unavailable }
            let list = EKCalendar(for: .reminder, eventStore: store)
            list.title = "Bridge Test"; list.source = source
            try store.saveCalendar(list, commit: true)
            lists = [list]
        }
        guard let list = lists.first else { return ["status": "verified", "records": [], "list_exists": false] }
        if verb == "fixture" && action == "burst" {
            guard list.allowsContentModifications else { throw TruthFailure.unavailable }
            let count = request["count"] as? Int ?? 200
            guard count > 0, count <= 200 else { throw TruthFailure.invalid }
            var effects: [[String: Any]] = []
            let clock = ContinuousClock()
            let start = clock.now
            for index in 0..<count {
                try await clock.sleep(until: start.advanced(by: .milliseconds(index * 200)))
                let reminder = EKReminder(eventStore: store)
                reminder.calendar = list
                reminder.title = tag + String(format: " live-%03d", index)
                try store.save(reminder, commit: true)
                effects.append(["title": reminder.title ?? "", "effect_uptime_ns": clock_gettime_nsec_np(CLOCK_UPTIME_RAW), "effect_wall_ns": Date().timeIntervalSince1970 * 1_000_000_000, "key": SHA256.hash(data: Data(reminder.calendarItemIdentifier.utf8)).map { String(format: "%02x", $0) }.joined()])
            }
            return ["status": "verified", "effects": effects]
        }
        let predicate = store.predicateForReminders(in: [list])
        let fetched: Fetch = await withCheckedContinuation { continuation in
            store.fetchReminders(matching: predicate) { result in continuation.resume(returning: Fetch(reminders: result)) }
        }
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess, let all = fetched.reminders else { throw TruthFailure.unavailable }
        var selected = all.filter { reminder in
            (tag.isEmpty || (reminder.title ?? "").contains(tag)) && (title.isEmpty || reminder.title == title)
        }
        if let identifier = request["id"] as? String { selected = selected.filter { $0.calendarItemIdentifier == identifier } }
        if verb == "fixture" {
            guard list.allowsContentModifications else { throw TruthFailure.unavailable }
            switch action {
            case "create":
                guard title.contains(tag) else { throw TruthFailure.invalid }
                let reminder = EKReminder(eventStore: store)
                reminder.calendar = list; reminder.title = title
                if request["recurring"] as? Bool == true {
                    reminder.dueDateComponents = Calendar.current.dateComponents([.year, .month, .day], from: Date())
                    reminder.addRecurrenceRule(EKRecurrenceRule(recurrenceWith: .daily, interval: 1, end: nil))
                }
                try store.save(reminder, commit: true)
                return ["status": "verified", "id": reminder.calendarItemIdentifier, "effect_done": ISO8601DateFormatter().string(from: Date()), "effect_uptime_ns": clock_gettime_nsec_np(CLOCK_UPTIME_RAW)]
            case "create_list": return ["status": "verified", "id": list.calendarIdentifier]
            case "cleanup_list":
                guard request["list_id"] as? String == list.calendarIdentifier, all.allSatisfy({ ($0.title ?? "").contains(tag) }) else { throw TruthFailure.invalid }
                try store.removeCalendar(list, commit: true)
            case "cleanup", "delete":
                for reminder in selected { try store.remove(reminder, commit: false) }
                try store.commit()
            case "modify", "complete", "uncomplete":
                guard selected.count == 1 else { throw TruthFailure.invalid }
                let reminder = selected[0]
                if action == "modify" {
                    guard let replacement = request["new_title"] as? String, replacement.contains(tag) else { throw TruthFailure.invalid }
                    reminder.title = replacement
                } else { reminder.isCompleted = action == "complete" }
                try store.save(reminder, commit: true)
            default: throw TruthFailure.invalid
            }
            return ["status": "verified"]
        }
        let records: [[String: Any]] = selected.map { reminder in
            var result: [String: Any] = ["id": reminder.calendarItemIdentifier, "title": reminder.title ?? "", "completed": reminder.isCompleted, "recurring": reminder.hasRecurrenceRules, "list": list.title]
            if let due = reminder.dueDateComponents { result["due"] = ["year": due.year ?? 0, "month": due.month ?? 0, "day": due.day ?? 0] }
            return result
        }
        return ["status": "verified", "list_exists": true, "list_id": list.calendarIdentifier, "records": records]
    }

    private static func messages(_ request: [String: Any]) throws -> [String: Any] {
        let guid = request["guid"] as? String ?? ""
        let tag = request["tag"] as? String ?? ""
        guard !guid.isEmpty || (tag.hasPrefix("[ab-test ") && tag.hasSuffix("]")) else { throw TruthFailure.invalid }
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if database != nil { sqlite3_close(database) }; throw TruthFailure.permissionMissing }
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        let query = "SELECT guid,text,is_from_me,is_delivered,error,date FROM message WHERE " + (guid.isEmpty ? "instr(text,?)>0" : "guid=?") + " ORDER BY ROWID LIMIT 1000"
        guard sqlite3_prepare_v2(database, query, -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        let binding = guid.isEmpty ? tag : guid
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        guard sqlite3_bind_text(statement, 1, binding, -1, transient) == SQLITE_OK else { throw TruthFailure.invalid }
        var rows: [[String: Any]] = []
        var status = sqlite3_step(statement)
        while status == SQLITE_ROW {
            let text = sqlite3_column_text(statement, 1).map { String(cString: $0) } ?? ""
            var row: [String: Any] = ["guid": sqlite3_column_text(statement, 0).map { String(cString: $0) } ?? "", "is_from_me": sqlite3_column_int(statement, 2) != 0, "is_delivered": sqlite3_column_int(statement, 3) != 0, "error": sqlite3_column_int(statement, 4), "date": sqlite3_column_int64(statement, 5)]
            if text.contains("[ab-test ") { row["text"] = text }
            rows.append(row)
            status = sqlite3_step(statement)
        }
        guard status == SQLITE_DONE else { throw TruthFailure.unavailable }
        return ["status": "verified", "records": rows]
    }
}
