import Foundation
@preconcurrency import EventKit
import SQLite3

private struct ReminderFetch: @unchecked Sendable { let reminders: [EKReminder]? }

@MainActor
struct CreatedContainers {
    static func run(_ verb: String, _ request: [String: Any]) async throws -> [String: Any] {
        switch (verb, request["action"] as? String) {
        case ("fixture", "remove_created"): return try await remove(request)
        case ("observe", "remaining"): return try remaining(request)
        default: throw TruthFailure.invalid
        }
    }

    private static func valid(_ tag: String) -> Bool {
        tag.hasPrefix("[ab-test ") && tag.hasSuffix("]") && tag.count > 11 && !tag.contains("\n")
    }

    private static func owned(_ title: String, tag: String) -> Bool {
        title == "Bridge Test " + tag || title.hasPrefix("Bridge Test " + tag + " ")
    }

    private static func entity(_ kind: String) throws -> EKEntityType? {
        switch kind {
        case "calendar": return .event
        case "reminder_list": return .reminder
        case "notes_folder": return nil
        default: throw TruthFailure.invalid
        }
    }

    private static func remove(_ request: [String: Any]) async throws -> [String: Any] {
        guard let kind = request["kind"] as? String, let id = request["id"] as? String, !id.isEmpty, let tag = request["tag"] as? String, valid(tag) else { throw TruthFailure.invalid }
        guard let type = try entity(kind) else { return try await removeFolder(key: id, tag: tag) }
        guard EKEventStore.authorizationStatus(for: type) == .fullAccess else { throw TruthFailure.permissionMissing }
        let store = EKEventStore()
        guard let container = store.calendar(withIdentifier: id) else { return ["status": "verified", "removed": false, "absent": true] }
        guard container.calendarIdentifier == id, container.allowedEntityTypes.contains(type == .event ? EKEntityMask.event : EKEntityMask.reminder), owned(container.title, tag: tag), container.allowsContentModifications, !container.isImmutable else { throw TruthFailure.invalid }
        let contents = type == .event ? events(store, in: container) : try await reminders(store, in: container)
        guard contents.allSatisfy({ $0.contains(tag) }) else { throw TruthFailure.invalid }
        try store.removeCalendar(container, commit: true)
        guard EKEventStore().calendar(withIdentifier: id) == nil else { throw TruthFailure.unavailable }
        return ["status": "verified", "removed": true, "absent": true, "removed_items": contents.count]
    }

    private static func events(_ store: EKEventStore, in calendar: EKCalendar) -> [String] {
        var bounds = Calendar(identifier: .gregorian)
        bounds.timeZone = .current
        let today = bounds.startOfDay(for: Date())
        var start = bounds.date(byAdding: .year, value: -10, to: today)!
        let end = bounds.date(byAdding: .year, value: 10, to: today)!
        var titles: [String] = []
        while start < end {
            let next = min(bounds.date(byAdding: .year, value: 1, to: start)!, end)
            titles += store.events(matching: store.predicateForEvents(withStart: start, end: next, calendars: [calendar])).map { $0.title ?? "" }
            start = next
        }
        return titles
    }

    private static func reminders(_ store: EKEventStore, in list: EKCalendar) async throws -> [String] {
        let predicate = store.predicateForReminders(in: [list])
        let fetched: ReminderFetch = await withCheckedContinuation { continuation in
            store.fetchReminders(matching: predicate) { result in continuation.resume(returning: ReminderFetch(reminders: result)) }
        }
        guard let found = fetched.reminders else { throw TruthFailure.unavailable }
        return found.map { $0.title ?? "" }
    }

    private static func removeFolder(key: String, tag: String) async throws -> [String: Any] {
        guard let folder = try notesFolders(keys: [key], tags: []).first else { return ["status": "verified", "removed": false, "absent": true] }
        guard owned(folder.title, tag: tag) else { throw TruthFailure.invalid }
        _ = try await AppleTruth.notes("fixture", ["source": "notes", "action": "cleanup", "tag": tag])
        _ = try await AppleTruth.notes("fixture", ["source": "notes", "action": "cleanup_created_folders", "created_by_run": true, "tag": tag, "folders": [["id": folder.id, "title": folder.title]]])
        guard try notesFolders(keys: [key], tags: []).isEmpty else { throw TruthFailure.unavailable }
        return ["status": "verified", "removed": true, "absent": true]
    }

    private static func remaining(_ request: [String: Any]) throws -> [String: Any] {
        guard let kinds = request["kinds"] as? [String], let tags = request["tags"] as? [String], tags.allSatisfy(valid), let created = request["created"] as? [[String: Any]] else { throw TruthFailure.invalid }
        var response: [String: Any] = ["status": "verified"]
        for kind in kinds {
            let ids = Set(created.filter { $0["kind"] as? String == kind }.compactMap { $0["id"] as? String })
            if let type = try entity(kind) {
                guard EKEventStore.authorizationStatus(for: type) == .fullAccess else { throw TruthFailure.permissionMissing }
                response[kind] = EKEventStore().calendars(for: type).filter { container in
                    ids.contains(container.calendarIdentifier) || tags.contains { container.title.contains($0) }
                }.count
            } else {
                response[kind] = try notesFolders(keys: Array(ids), tags: tags).count
            }
        }
        return response
    }

    private static func notesFolders(keys: [String], tags: [String]) throws -> [(id: String, title: String)] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if db != nil { sqlite3_close(db) }; throw TruthFailure.permissionMissing
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 1000)
        let matches = keys.map { _ in "f.ZIDENTIFIER=?" } + tags.map { _ in "instr(f.ZTITLE2,?)>0" }
        guard !matches.isEmpty else { return [] }
        let sql = "SELECT m.Z_UUID,f.Z_PK,COALESCE(f.ZTITLE2,'') FROM Z_METADATA m CROSS JOIN ZICCLOUDSYNCINGOBJECT f WHERE f.Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICFolder') AND COALESCE(f.ZMARKEDFORDELETION,0)=0 AND (" + matches.joined(separator: " OR ") + ")"
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        for (index, value) in (keys + tags).enumerated() {
            let bound = value.withCString { sqlite3_bind_text(statement, Int32(index + 1), $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard bound == SQLITE_OK else { throw TruthFailure.unavailable }
        }
        var folders: [(id: String, title: String)] = []
        while true {
            let result = sqlite3_step(statement)
            if result == SQLITE_DONE { return folders }
            guard result == SQLITE_ROW, let uuid = sqlite3_column_text(statement, 0), let title = sqlite3_column_text(statement, 2) else { throw TruthFailure.unavailable }
            folders.append((id: "x-coredata://" + String(cString: uuid) + "/ICFolder/p" + String(sqlite3_column_int64(statement, 1)), title: String(cString: title)))
        }
    }
}
