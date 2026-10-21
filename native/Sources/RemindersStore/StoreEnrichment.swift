import Foundation
import SourceSupport

public actor RemindersStoreEnrichment {
    private var sessions: [String: RemindersStoreSession] = [:]
    private var snapshots: [String: TypedStoreSnapshot] = [:]
    private var fields: [String: [String: JSON]] = [:]
    private var base: [String: JSON] = [:]
    private var generation = ""
    private var refreshing = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private let trace: CaptureTrace?
    private let paths: @Sendable () throws -> [String]

    public init(paths: @escaping @Sendable () throws -> [String] = { try RemindersStore.paths() }, trace: CaptureTrace? = nil) { self.paths = paths; self.trace = trace }

    public func refresh(_ observations: [JSON], generation: String) async -> [JSON] {
        let queued = CaptureTrace.now()
        if refreshing { await withCheckedContinuation { waiters.append($0) } }
        let started = CaptureTrace.now()
        trace?.record("reminders_store_queue", fields: ["started_ns": .integer(queued), "finished_ns": .integer(started)])
        refreshing = true
        defer {
            if waiters.isEmpty { refreshing = false }
            else { waiters.removeFirst().resume() }
        }
        if self.generation != generation { base = [:]; self.generation = generation }
        var touched: Set<String> = []
        do {
            let current = Set(try paths())
            for path in Set(sessions.keys).subtracting(current) {
                touched.formUnion(fields[path]?.keys.map { $0 } ?? [])
                fields.removeValue(forKey: path)
                snapshots.removeValue(forKey: path)
                sessions.removeValue(forKey: path)
            }
            for path in current.sorted() {
                do {
                    if sessions[path] == nil { sessions[path] = try RemindersStoreSession(path: path) }
                    let readStarted = CaptureTrace.now()
                    let readStartedWall = Date().timeIntervalSince1970 * 1_000_000_000
                    let result = try await sessions[path]!.read()
                    trace?.record("reminders_store_read", fields: ["started_ns": .integer(readStarted), "finished_ns": .integer(CaptureTrace.now()), "started_wall_ns": .number(readStartedWall), "finished_wall_ns": .number(Date().timeIntervalSince1970 * 1_000_000_000), "mode": .string(result.mode), "decoded_rows": .number(Double(result.decodedRows)), "duration_ms": .number(result.timings.totalMS)])
                    let previous = snapshots[path] ?? TypedStoreSnapshot()
                    let snapshot = result.snapshot
                    var changedReminders = result.reminderKeys
                    let changedParents = result.reminderKeys.filter { RemindersStoreProjection.reminderIdentifier(previous.reminders[$0]) != RemindersStoreProjection.reminderIdentifier(snapshot.reminders[$0]) }
                    if !changedParents.isEmpty {
                        changedReminders.formUnion(snapshot.reminders.values.filter { $0.parent.map(changedParents.contains) == true }.map { $0.identity.key })
                    }
                    for key in changedReminders {
                        let old = RemindersStoreProjection.reminderIdentifier(previous.reminders[key]).map { "reminders.reminder/" + $0 }
                        let native = RemindersStoreProjection.reminderIdentifier(snapshot.reminders[key]).map { "reminders.reminder/" + $0 }
                        let value = snapshot.reminders[key].map { RemindersStoreProjection.reminder($0, snapshot: snapshot, columns: result.columns["ZREMCDREMINDER"] ?? []) }
                        update(path: path, old: old, native: native, value: value, touched: &touched)
                    }
                    var changedLists = result.listKeys
                    let changedGroups = result.listKeys.filter { previous.lists[$0].flatMap { RemindersStoreProjection.identifier($0.identity) } != snapshot.lists[$0].flatMap { RemindersStoreProjection.identifier($0.identity) } }
                    if !changedGroups.isEmpty {
                        changedLists.formUnion(snapshot.lists.values.filter { $0.parent.map(changedGroups.contains) == true || $0.alternateParent.map(changedGroups.contains) == true }.map { $0.identity.key })
                    }
                    for key in changedLists {
                        let old = previous.lists[key].flatMap { RemindersStoreProjection.identifier($0.identity) }.map { "reminders.list/" + $0 }
                        let native = snapshot.lists[key].flatMap { RemindersStoreProjection.identifier($0.identity) }.map { "reminders.list/" + $0 }
                        let value = snapshot.lists[key].map { RemindersStoreProjection.list($0, snapshot: snapshot, columns: result.columns["ZREMCDBASELIST"] ?? []) }
                        update(path: path, old: old, native: native, value: value, touched: &touched)
                    }
                    snapshots[path] = snapshot
                } catch {
                    trace?.record("reminders_store_read_failed")
                    touched.formUnion(fields[path]?.keys.map { $0 } ?? [])
                    fields.removeValue(forKey: path)
                    snapshots.removeValue(forKey: path)
                    sessions.removeValue(forKey: path)
                }
            }
        } catch {
            touched.formUnion(base.keys)
            fields = [:]; snapshots = [:]; sessions = [:]
        }
        var outgoing: [String: JSON] = [:]
        for observation in observations {
            let identity = Self.key(observation)
            outgoing[identity] = observation
            if observation["kind"].string == "upsert" { base[identity] = RemindersStoreProjection.baseObservation(observation) }
            else if observation["kind"].string == "absent" { base.removeValue(forKey: identity) }
        }
        for key in touched where outgoing[key] == nil {
            if var observation = base[key]?.object {
                observation["observed_at"] = .string(timestamp())
                outgoing[key] = .object(observation)
            }
        }
        return apply(outgoing.keys.sorted().compactMap { outgoing[$0] })
    }

    public func apply(_ observations: [JSON]) -> [JSON] {
        observations.map { observation in
            guard observation["kind"].string == "upsert", ["reminders.list", "reminders.reminder"].contains(observation["collection"].string) else { return observation }
            let matches = fields.values.compactMap { $0[Self.key(observation)] }
            let enrichment = matches.count == 1 ? matches[0] : RemindersStoreProjection.unavailable(observation["collection"].string)
            var result = observation.object
            result["data"] = .object(RemindersStoreProjection.baseData(observation["data"]).object.merging(enrichment.object) { _, new in new })
            return .object(result)
        }
    }

    private func update(path: String, old: String?, native: String?, value: JSON?, touched: inout Set<String>) {
        if let old, old != native { fields[path]?.removeValue(forKey: old); touched.insert(old) }
        if let native, let value, fields[path]?[native] != value {
            fields[path, default: [:]][native] = value
            touched.insert(native)
        }
    }

    private static func key(_ observation: JSON) -> String { observation["collection"].string + "/" + observation["native_key"].string }
}
