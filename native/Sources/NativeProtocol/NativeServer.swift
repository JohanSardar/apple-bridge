import Foundation
import SourceSupport

public struct NativeFacts: Sendable {
    public let items: [JSON]
    public let scope: JSON
    public let generation: String
    public let complete: Bool
    public init(items: [JSON], scope: JSON, generation: String, complete: Bool) {
        self.items = items; self.scope = scope; self.generation = generation; self.complete = complete
    }
}

public protocol NativeAdapter: Sendable {
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts
    func revalidate(_ operation: String, identities: [JSON]) async throws -> [String]
    func forwardChanges(to server: NativeServer) async
}

public extension NativeAdapter {
    func revalidate(_ operation: String, identities: [JSON]) async throws -> [String] { throw SourceFailure.unsupported }
    func forwardChanges(to server: NativeServer) async {}
}

public actor NativeServer {
    public static let factOperations: Set<String> = ["agenda.calendars", "agenda.events", "agenda.reminder_lists", "agenda.reminders", "contacts.containers", "contacts.groups", "contacts.contacts", "files.ubiquity", "mac.state", "messages.media", "focus.status", "safari.tabs", "accessibility.trusted", "mac.pasteboard", "calendar.candidates", "calendar.account"]
    public static let maximumUbiquityPaths = 500
    public static let revalidationCollections: [String: String] = ["agenda.calendars": "calendar.calendar", "agenda.reminder_lists": "reminders.list", "agenda.reminders": "reminders.reminder", "contacts.containers": "contacts.container", "contacts.groups": "contacts.group", "contacts.contacts": "contacts.contact"]
    public static let maximumRevalidation = 500
    private struct Snapshot {
        let operation: String
        let args: JSON
        let facts: NativeFacts
        let expiry: Date
    }
    private let adapter: any NativeAdapter
    private var fence: UInt64 = 0
    private var opened = false
    private var inflight: Set<String> = []
    private var wire: NativeWire?
    private var snapshots: [String: Snapshot] = [:]

    public init(adapter: any NativeAdapter) { self.adapter = adapter }

    public func handle(_ frame: JSON) async -> JSON {
        do {
            guard case .object = frame else { throw SourceFailure.invalid }
            if frame["t"].string == "Hello" {
                guard !opened, frame["protocol"].string == "native/1", frame["version"] == .integer(1), frame["owner_fence"].unsigned > 0 else { throw SourceFailure.invalid }
                fence = frame["owner_fence"].unsigned; opened = true
                return .object(["t": .string("Welcome"), "protocol": .string("native/1"), "version": .integer(1)])
            }
            guard opened, !frame["id"].string.isEmpty, case .object = frame["args"] else { throw SourceFailure.invalid }
            let requestID = frame["id"].string
            guard inflight.insert(requestID).inserted else { throw SourceFailure.conflict }
            defer { inflight.remove(requestID) }
            switch frame["t"].string {
            case "Facts": return try await page(frame)
            case "Revalidate": return try await revalidate(frame)
            default: throw SourceFailure.unsupported
            }
        } catch {
            let code: String
            switch error as? SourceFailure {
            case .permissionMissing: code = "permission_missing"
            case .invalid: code = "invalid"
            case .unsupported: code = "unsupported"
            case .expired: code = "expired"
            case .conflict: code = "conflict"
            case .corrupt: code = "corrupt"
            default: code = "unavailable"
            }
            return .object(["t": .string("Error"), "id": frame["id"], "code": .string(code)])
        }
    }

    private func revalidate(_ frame: JSON) async throws -> JSON {
        let op = frame["op"].string
        guard let collection = Self.revalidationCollections[op] else { throw SourceFailure.unsupported }
        guard Set(frame["args"].object.keys) == ["identities"] else { throw SourceFailure.invalid }
        let identities = frame["args"]["identities"].array
        guard !identities.isEmpty, identities.count <= Self.maximumRevalidation,
              identities.allSatisfy({ $0["collection"].string == collection && !$0["native_key"].string.isEmpty }) else { throw SourceFailure.invalid }
        let states = try await adapter.revalidate(op, identities: identities)
        guard states.count == identities.count, states.allSatisfy({ [RevalidationState.present, RevalidationState.missing, RevalidationState.outsideScope].contains($0) }) else { throw SourceFailure.corrupt }
        let results = zip(identities, states).map { identity, state in JSON.object(["collection": identity["collection"], "native_key": identity["native_key"], "state": .string(state)]) }
        return .object(["t": .string("Revalidated"), "id": frame["id"], "op": .string(op), "results": .array(results)])
    }

    public func changed(source: String, generation: String) {
        try? wire?.send(.object(["t": .string("Changed"), "source": .string(source), "generation": .string(generation)]))
    }

    private func page(_ frame: JSON) async throws -> JSON {
        let op = frame["op"].string
        guard Self.factOperations.contains(op) else { throw SourceFailure.unsupported }
        let limit = frame["limit"] == .null ? 100 : frame["limit"].unsigned
        guard limit > 0, limit <= 1000 else { throw SourceFailure.invalid }
        snapshots = snapshots.filter { $0.value.expiry > Date() }
        let token: String
        let offset: Int
        let snapshot: Snapshot
        if frame["snapshot"] == .null {
            guard frame["cursor"] == .null, snapshots.count < 8 else { throw SourceFailure.invalid }
            let facts = try await adapter.facts(op, args: frame["args"])
            guard !facts.generation.isEmpty else { throw SourceFailure.invalid }
            switch facts.scope {
            case .string(let scope): guard !scope.isEmpty else { throw SourceFailure.invalid }
            case .object(let scope): guard !scope.isEmpty else { throw SourceFailure.invalid }
            default: throw SourceFailure.invalid
            }
            guard facts.items.count <= 100000, try JSONEncoder().encode(JSON.array(facts.items)).count <= 32 * 1024 * 1024 else { throw SourceFailure.unavailable }
            token = UUID().uuidString; offset = 0
            snapshot = Snapshot(operation: op, args: frame["args"], facts: facts, expiry: Date().addingTimeInterval(60))
            snapshots[token] = snapshot
        } else {
            token = frame["snapshot"].string
            guard let stored = snapshots[token] else { throw SourceFailure.expired }
            guard stored.operation == op, stored.args == frame["args"], frame["cursor"].string.hasPrefix(token + ":"), let index = Int(frame["cursor"].string.dropFirst(token.count + 1)), index >= 0, index <= stored.facts.items.count else { throw SourceFailure.invalid }
            snapshot = stored; offset = index
        }
        var end = min(offset + Int(limit), snapshot.facts.items.count)
        func result(_ end: Int) -> JSON {
            let terminal = end == snapshot.facts.items.count
            return .object(["t": .string("FactPage"), "id": frame["id"], "op": .string(op), "snapshot": .string(token), "items": .array(Array(snapshot.facts.items[offset..<end])), "cursor": terminal ? .null : .string(token + ":" + String(end)), "complete": .bool(terminal && snapshot.facts.complete), "scope": snapshot.facts.scope, "generation": .string(snapshot.facts.generation)])
        }
        while try JSONEncoder().encode(result(end)).count > NativeWire.maximumFrame {
            guard end > offset + 1 else { snapshots.removeValue(forKey: token); throw SourceFailure.unavailable }
            end = offset + (end - offset) / 2
        }
        let page = result(end)
        if end == snapshot.facts.items.count { snapshots.removeValue(forKey: token) }
        return page
    }

    public func run(descriptor: Int32) async throws {
        let link = try NativeWire(descriptor: descriptor)
        wire = link
        defer { wire = nil }
        try await withThrowingTaskGroup(of: Void.self) { group in
            while !Task.isCancelled {
                let frame = try await Task.detached { try link.receive() }.value
                let hello = frame["t"].string == "Hello"
                guard hello == !opened else { throw SourceFailure.invalid }
                if hello {
                    try link.send(await handle(frame))
                    if !opened { throw SourceFailure.invalid }
                } else {
                    group.addTask { try link.send(await self.handle(frame)) }
                }
            }
            group.cancelAll()
        }
    }
}
