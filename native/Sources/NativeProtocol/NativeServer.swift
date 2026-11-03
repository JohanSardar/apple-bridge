import Foundation
import CryptoKit
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
    func effectsAllowed() async -> Bool
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts
    func effect(_ operation: String, args: JSON, permit: JSON) async throws -> JSON
}

public extension NativeAdapter {
    func effectsAllowed() async -> Bool { true }
}

public actor NativeServer {
    public static let factOperations: Set<String> = ["agenda.calendars", "agenda.events", "agenda.reminder_lists", "agenda.reminders", "contacts.containers", "contacts.groups", "contacts.contacts", "contacts.history", "ui.target", "ui.actions", "focus.status", "mac.state", "location.status", "permissions.status"]
    public static let effectOperations: Set<String> = [
        "calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar",
        "reminders.create", "reminders.update", "reminders.complete", "reminders.delete", "reminders.create_list", "reminders.rename_list", "reminders.delete_list", "reminders.uncomplete",
        "contacts.create", "contacts.update", "contacts.delete", "contacts.create_group", "contacts.rename_group", "contacts.delete_group", "contacts.add_member", "contacts.remove_member",
        "messages.send", "messages.reply", "messages.react", "messages.edit", "messages.unsend", "messages.mark_read", "messages.mark_unread", "messages.delete_message", "messages.delete_chat",
        "media.derive", "vision.ocr", "pdf.text", "pdf.search", "pdf.render", "pdf.annotate", "audio.transcribe", "speech.synthesize", "speech.say", "capture.play_sound",
        "maps.snapshot", "maps.search", "maps.nearby", "maps.directions", "maps.eta", "maps.open", "location.current", "location.geocode", "location.reverse_geocode",
        "capture.screenshot", "capture.camera_photo", "capture.record_audio", "mac.screen_record", "print.document", "focus.set", "shortcuts.run", "shortcuts.actions",
        "mac.set_volume", "mac.set_mute", "mac.set_appearance", "mac.launch_app", "mac.activate_app", "mac.quit_app", "mac.keep_awake", "mac.clipboard_write", "mac.clipboard_read", "mac.notify", "mac.open_settings", "mac.set_wallpaper", "mac.network_check", "mac.bluetooth_devices", "mac.time_machine_status", "mac.recognize_song", "mac.iwork_export"
    ]
    private struct Snapshot {
        let operation: String
        let args: JSON
        let facts: NativeFacts
        let expiry: Date
    }
    private let adapter: any NativeAdapter
    private let files: NativeFiles?
    private var fence: UInt64 = 0
    private var opened = false
    private var active = false
    private var usedNonces: Set<String> = []
    private var usedCommands: Set<String> = []
    private var snapshots: [String: Snapshot] = [:]

    public init(adapter: any NativeAdapter, files: NativeFiles? = nil) { self.adapter = adapter; self.files = files }

    public static func requestHash(operation: String, args: JSON) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let bytes = try encoder.encode(args)
        return SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    public func handle(_ frame: JSON) async -> JSON {
        do {
            guard case .object = frame else { throw SourceFailure.invalid }
            if frame["t"].string == "Hello" {
                guard !opened, frame["protocol"].string == "native/1", frame["version"] == .integer(1), frame["owner_fence"].unsigned > 0 else { throw SourceFailure.invalid }
                fence = frame["owner_fence"].unsigned; opened = true
                return .object(["t": .string("Welcome"), "protocol": .string("native/1"), "version": .integer(1)])
            }
            guard opened, !frame["id"].string.isEmpty, case .object = frame["args"] else { throw SourceFailure.invalid }
            guard !active else { throw SourceFailure.conflict }
            active = true
            defer { active = false }
            switch frame["t"].string {
            case "Release":
                guard let files else { throw SourceFailure.unsupported }
                try await files.release(frame["args"]["file_ref"])
                return .object(["t": .string("Released"), "id": frame["id"]])
            case "Facts": return try await page(frame)
            case "Effect":
                let op = frame["op"].string
                guard Self.effectOperations.contains(op) else { throw SourceFailure.unsupported }
                guard await adapter.effectsAllowed() else { throw SourceFailure.conflict }
                let permit = frame["permit"]
                guard permit["operation"].string == op, permit["owner_fence"].unsigned == fence,
                      !permit["command_id"].string.isEmpty, !permit["ticket"].string.isEmpty, permit["nonce"].string.count == 64, permit["nonce"].string.allSatisfy({ "0123456789abcdef".contains($0) }),
                      permit["request_hash"].string == (try Self.requestHash(operation: op, args: frame["args"])) else { throw SourceFailure.invalid }
                guard case .integer(let expires) = permit["expires_at"], expires > UInt64(Date().timeIntervalSince1970) else { throw SourceFailure.expired }
                guard !usedNonces.contains(permit["nonce"].string), !usedCommands.contains(permit["command_id"].string), usedNonces.count < 10000 else { throw SourceFailure.conflict }
                usedNonces.insert(permit["nonce"].string); usedCommands.insert(permit["command_id"].string)
                var result: JSON
                do { result = try await adapter.effect(op, args: frame["args"], permit: permit) }
                catch {
                    result = .object(["disposition": .string("indeterminate"), "observed": .bool(false), "reason": .string("native_effect_evidence_unavailable")])
                }
                let encoded = try JSONEncoder().encode(result)
                if encoded.count > 240000 {
                    guard let files else { throw SourceFailure.unavailable }
                    result = .object(["file_ref": try await files.write(encoded, mime: "application/json")])
                }
                return .object(["t": .string("EffectResult"), "id": frame["id"], "evidence": result])
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
        let wire = try NativeWire(descriptor: descriptor)
        while !Task.isCancelled {
            let frame = try await Task.detached { try wire.receive() }.value
            guard (frame["t"].string == "Hello") == !opened else { throw SourceFailure.invalid }
            let response = await handle(frame)
            try wire.send(response)
            if !opened { throw SourceFailure.invalid }
        }
    }
}
