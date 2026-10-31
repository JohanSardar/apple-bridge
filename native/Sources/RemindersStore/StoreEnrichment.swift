import Foundation
import SourceSupport

public actor RemindersStoreEnrichment {
    private var sessions: [String: RemindersStoreSession] = [:]
    private var manualOrders: [String: [Int64: StoreManualOrder]] = [:]
    private var sectionStates: [String: [Int64: StoreSections]] = [:]
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
                sectionStates.removeValue(forKey: path)
                manualOrders.removeValue(forKey: path)
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
                    let changedOrderLists = result.listKeys.filter { snapshot.lists[$0] == nil || previous.lists[$0]?.orderingJSON != snapshot.lists[$0]?.orderingJSON || manualOrders[path]?[$0] == nil }
                    var changedPositions: [Int64: Set<UUID>] = [:]
                    var changedOrderCoverage: Set<Int64> = []
                    for key in changedOrderLists {
                        let before = manualOrders[path]?[key] ?? .unsupported
                        let after = StoreManualOrder.decode(snapshot.lists[key]?.orderingJSON)
                        if let changed = after.changedIdentifiers(from: before) {
                            if !changed.isEmpty { changedPositions[key] = changed }
                        } else { changedOrderCoverage.insert(key) }
                        if snapshot.lists[key] == nil { manualOrders[path]?.removeValue(forKey: key) }
                        else { manualOrders[path, default: [:]][key] = after }
                    }
                    if !changedPositions.isEmpty || !changedOrderCoverage.isEmpty {
                        for row in snapshot.reminders.values {
                            guard let list = row.list else { continue }
                            if changedOrderCoverage.contains(list) { changedReminders.insert(row.identity.key) }
                            else if let changed = changedPositions[list],
                                    let identifier = row.identity.uuid ?? row.identity.textIdentifier.flatMap(UUID.init(uuidString:)),
                                    changed.contains(identifier) { changedReminders.insert(row.identity.key) }
                        }
                    }
                    var sectionLists = Set(result.listKeys.filter { previous.lists[$0]?.sectionOrder != snapshot.lists[$0]?.sectionOrder || previous.lists[$0]?.sectionMembership != snapshot.lists[$0]?.sectionMembership })
                    for key in result.reminderKeys where previous.reminders[key]?.list != snapshot.reminders[key]?.list || RemindersStoreProjection.reminderIdentifier(previous.reminders[key]) != RemindersStoreProjection.reminderIdentifier(snapshot.reminders[key]) {
                        if let list = previous.reminders[key]?.list { sectionLists.insert(list) }
                        if let list = snapshot.reminders[key]?.list { sectionLists.insert(list) }
                    }
                    for key in result.sectionKeys {
                        if let list = previous.sections[key]?.list { sectionLists.insert(list) }
                        if let list = snapshot.sections[key]?.list { sectionLists.insert(list) }
                    }
                    for key in result.listKeys.union(sectionLists) {
                        sectionStates[path, default: [:]][key] = snapshot.lists[key].flatMap { StoreSections.validated($0, snapshot: snapshot) }
                    }
                    changedReminders.formUnion(snapshot.reminders.values.filter { $0.list.map(sectionLists.contains) == true }.map { $0.identity.key })
                    let changedParents = result.reminderKeys.filter { RemindersStoreProjection.reminderIdentifier(previous.reminders[$0]) != RemindersStoreProjection.reminderIdentifier(snapshot.reminders[$0]) }
                    if !changedParents.isEmpty {
                        changedReminders.formUnion(snapshot.reminders.values.filter { $0.parent.map(changedParents.contains) == true }.map { $0.identity.key })
                    }
                    if !result.objectKeys.isEmpty || !result.labelKeys.isEmpty {
                        for key in result.objectKeys.union(Set(snapshot.objects.values.filter { $0.hashtagLabel.map(result.labelKeys.contains) == true }.map { $0.identity.key })) {
                            for object in [previous.objects[key], snapshot.objects[key]].compactMap({ $0 }) {
                                changedReminders.formUnion([object.reminder, object.reminder1, object.reminder2, object.reminder3, object.reminder4, object.reminder5].compactMap { $0 })
                            }
                        }
                    }
                    for key in changedReminders {
                        let old = RemindersStoreProjection.reminderIdentifier(previous.reminders[key]).map { "reminders.reminder/" + $0 }
                        let native = RemindersStoreProjection.reminderIdentifier(snapshot.reminders[key]).map { "reminders.reminder/" + $0 }
                        let value = snapshot.reminders[key].map { RemindersStoreProjection.reminder($0, snapshot: snapshot, columns: result.columns["ZREMCDREMINDER"] ?? [], manualOrder: $0.list.flatMap { manualOrders[path]?[$0] } ?? .unsupported, sections: $0.list.flatMap { sectionStates[path]?[$0] }) }
                        update(path: path, old: old, native: native, value: value, touched: &touched)
                    }
                    var changedLists = result.listKeys.union(sectionLists)
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
                    sectionStates.removeValue(forKey: path)
                    manualOrders.removeValue(forKey: path)
                    sessions.removeValue(forKey: path)
                }
            }
        } catch {
            touched.formUnion(base.keys)
            fields = [:]; snapshots = [:]; manualOrders = [:]; sectionStates = [:]; sessions = [:]
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
