import Foundation
import SourceSupport

public actor RemindersCaptureScope {
    private var sessions: [String: RemindersStoreSession] = [:]
    private var reading = false
    private var waiting: [CheckedContinuation<Void, Never>] = []
    private var snapshots: [String: TypedStoreSnapshot] = [:]
    private let paths: @Sendable () throws -> [String]

    public init(paths: @escaping @Sendable () throws -> [String] = { try RemindersStore.paths() }) { self.paths = paths }

    public func read(trace: CaptureTrace? = nil) async -> Set<String>? {
        if reading { await withCheckedContinuation { waiting.append($0) } }
        reading = true
        defer {
            if waiting.isEmpty { reading = false }
            else { waiting.removeFirst().resume() }
        }
        do {
            let paths = Set(try paths())
            guard !paths.isEmpty else { return nil }
            var full = paths != Set(sessions.keys)
            var lists: Set<String> = []
            for path in Set(sessions.keys).subtracting(paths) {
                sessions.removeValue(forKey: path)
                snapshots.removeValue(forKey: path)
            }
            for path in paths.sorted() {
                if sessions[path] == nil { sessions[path] = try RemindersStoreSession(path: path) }
                guard let session = sessions[path] else { return nil }
                let result = try await session.read()
                let before = snapshots[path] ?? TypedStoreSnapshot()
                let after = result.snapshot
                snapshots[path] = after
                if result.mode == "full" { full = true; continue }
                for key in result.listKeys {
                    for row in [before.lists[key], after.lists[key]].compactMap({ $0 }) {
                        guard let identifier = RemindersStoreProjection.identifier(row.identity) else { full = true; continue }
                        lists.insert(identifier)
                    }
                }
                for key in result.reminderKeys {
                    for snapshot in [before, after] where snapshot.reminders[key] != nil {
                        if !Self.includeReminder(key, snapshot: snapshot, lists: &lists) { full = true }
                    }
                }
                for key in result.sectionKeys where before.sections[key] != after.sections[key] {
                    for snapshot in [before, after] {
                        guard let row = snapshot.sections[key] else { continue }
                        let owners = Set([row.list, row.smartList].compactMap { $0 }.filter { $0 != 0 })
                        if owners.isEmpty { full = true }
                        for owner in owners where !Self.includeList(owner, snapshot: snapshot, lists: &lists) { full = true }
                    }
                }
                for key in result.objectKeys where before.objects[key] != after.objects[key] {
                    for snapshot in [before, after] {
                        guard let row = snapshot.objects[key] else { continue }
                        let owners = Set([row.reminder, row.reminder1, row.reminder2, row.reminder3, row.reminder4, row.reminder5].compactMap { $0 }.filter { $0 != 0 })
                        let list = row.list.flatMap { $0 == 0 ? nil : $0 }
                        var owned = !owners.isEmpty || list != nil
                        for owner in owners where !Self.includeReminder(owner, snapshot: snapshot, lists: &lists) { owned = false }
                        if let list, !Self.includeList(list, snapshot: snapshot, lists: &lists) { owned = false }
                        if !owned {
                            full = true
                            trace?.record("reminders_scope_fallback", fields: ["entity": .string(result.entities[row.identity.entity] ?? "unknown"), "reason": .string("unresolved_object_owner")])
                        }
                    }
                }
                if result.labelKeys.contains(where: { before.labels[$0] != after.labels[$0] }) { full = true }
            }
            return full ? nil : lists
        } catch {
            sessions.removeAll()
            snapshots.removeAll()
            return nil
        }
    }

    private static func includeReminder(_ key: Int64, snapshot: TypedStoreSnapshot, lists: inout Set<String>) -> Bool {
        guard let row = snapshot.reminders[key], RemindersStoreProjection.reminderIdentifier(row) != nil,
              let list = row.list else { return false }
        return includeList(list, snapshot: snapshot, lists: &lists)
    }

    private static func includeList(_ key: Int64, snapshot: TypedStoreSnapshot, lists: inout Set<String>) -> Bool {
        guard let row = snapshot.lists[key], let identifier = RemindersStoreProjection.identifier(row.identity) else { return false }
        lists.insert(identifier)
        return true
    }
}
