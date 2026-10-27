import Foundation
import AppKit
import ApplicationServices
import Decoders
import SourceSupport

public enum MessagesParity {
    public static let operations = ["pin", "mute", "set_read_receipts", "recover_chat", "delete_chat_permanently"].map { "messages." + $0 }
    public static func failureReason(_ error: Error) -> String {
        if let failure = error as? SourceFailure { return String(describing: failure) }
        if let failure = error as? ProbeFailure { return String(describing: failure) }
        if let failure = error as? MessagesUIFailure { return failure.rawValue }
        let failure = error as NSError
        return failure.domain + ":" + String(failure.code)
    }
    public static var reportURL: URL { PrivateReport.directory.appendingPathComponent("messages-parity-selftest.json") }
    public static func report() -> JSON {
        guard let data = try? Data(contentsOf: reportURL), let value = try? JSONDecoder().decode(JSON.self, from: data), value["os_build"].string == ProcessInfo.processInfo.operatingSystemVersionString else { return .null }
        return value
    }
    public static func coverage() -> [JSON] {
        let ledger = report()
        let steps = ledger["steps"].array
        return operations.map { operation in
            let step = steps.first { $0["operation"].string == operation }
            return .object(["operation": .string(operation), "proven": .bool(operation == "messages.mute" && step?["proven"].bool == true && ledger["cleanup_state"].string == "verified_clean" && ["iMessage", "SMS", "RCS"].contains(step?["service"].string ?? "")), "reason": .string(step?["reason"].string ?? "parity_selftest_required")])
        }
    }
    static func properties(_ chat: String) throws -> [String: Any] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let row = try MessagesReader(path: path).query("SELECT properties FROM chat WHERE guid=?", [chat]).first
        guard let row else { throw SourceFailure.conflict }
        guard let data = row.blobs["properties"] else { return [:] }
        guard let decoded = MessageArchive.decode(data) else { throw SourceFailure.corrupt }
        return decoded
    }
    @UIActor
    static func menu(_ prepared: Prepared) throws -> (MessagesUISession, Set<String>) {
        let session = try MessagesAccessibility.uiSession(), root = session.root
        let row = try MessagesAccessibility.target(root, prepared: prepared)
        try MessagesAccessibility.contextMenu(row, session: session)
        var budget = 4000
        let items = MessagesAccessibility.matching(root, budget: &budget) { MessagesAccessibility.value($0, kAXRoleAttribute) == kAXMenuItemRole }
        guard budget > 0 else { throw SourceFailure.conflict }
        return (session, Set(items.map { MessagesAccessibility.value($0, kAXTitleAttribute) }))
    }
    @UIActor
    static func dismiss(_ session: MessagesUISession) throws { try session.press(MessagesUISession.escape) }
    @UIActor
    static func mute(_ prepared: Prepared, value: Bool) throws {
        let (session, titles) = try menu(prepared)
        let action = value ? "Hide Alerts" : "Show Alerts"
        let opposite = value ? "Show Alerts" : "Hide Alerts"
        guard titles.contains(action) != titles.contains(opposite) else { try? dismiss(session); throw SourceFailure.capabilityUnproven }
        if titles.contains(action) { try MessagesAccessibility.press(session, title: action) } else { try dismiss(session) }
        let deadline = Date().addingTimeInterval(3)
        repeat {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            let (menuSession, after) = try menu(prepared)
            try dismiss(menuSession)
            if after.contains(opposite) && !after.contains(action) { return }
        } while Date() < deadline
        throw SourceFailure.conflict
    }
    @UIActor
    static func proveMute(_ prepared: Prepared, titles: Set<String>) -> JSON {
        let operation = "messages.mute"
        let current = titles.contains("Show Alerts")
        guard titles.contains("Hide Alerts") != current else { return .object(["operation": .string(operation), "proven": .bool(false), "reason": .string("conversation_menu_target_unavailable")]) }
        var changed = false
        do {
            let before = try properties(prepared.action["native_key"].string)
            changed = true
            try mute(prepared, value: !current)
            let toggled = try properties(prepared.action["native_key"].string)
            try mute(prepared, value: current)
            changed = false
            let restored = try properties(prepared.action["native_key"].string)
            let keys = Set(before.keys).union(toggled.keys).union(restored.keys).filter { key in
                guard let active = toggled[key] as? NSNumber, CFGetTypeID(active) == CFBooleanGetTypeID(), let reset = restored[key] as? NSNumber, CFGetTypeID(reset) == CFBooleanGetTypeID(), active.boolValue != reset.boolValue else { return false }
                return before[key] == nil || (before[key] as? NSNumber)?.boolValue == reset.boolValue
            }
            guard keys.count == 1, let key = keys.first, let active = toggled[key] as? NSNumber else { throw SourceFailure.capabilityUnproven }
            return .object(["operation": .string(operation), "proven": .bool(true), "reason": .string("native_menu_and_properties_roundtrip"), "property": .string(key), "muted_value": .bool(current ? !active.boolValue : active.boolValue), "service": prepared.nativeState["service"], "missing_is_unmuted": .bool(before[key] == nil && !current)])
        } catch {
            if changed { try? mute(prepared, value: current) }
            return .object(["operation": .string(operation), "proven": .bool(false), "reason": .string("mute_roundtrip_" + failureReason(error))])
        }
    }
    @UIActor
    public static func probe(_ prepared: Prepared) throws -> [JSON] {
        let operations = Self.operations
        func limited(_ reason: String) -> [JSON] {
            operations.map { .object(["operation": .string($0), "proven": .bool(false), "reason": .string(reason)]) }
        }
        do {
            try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string)
            let session = try MessagesAccessibility.uiSession(), root = session.root
            let row = try MessagesAccessibility.target(root, prepared: prepared)
            try MessagesAccessibility.contextMenu(row, session: session)
            var budget = 4000
            let items = MessagesAccessibility.matching(root, budget: &budget) { MessagesAccessibility.value($0, kAXRoleAttribute) == kAXMenuItemRole }
            let titles = Set(items.map { MessagesAccessibility.value($0, kAXTitleAttribute) })
            guard budget > 0 else { throw SourceFailure.conflict }
            try dismiss(session)
            var steps: [JSON] = []
            for (operation, labels, reason) in [
                ("messages.pin", ["Pin", "Unpin"], "pinned_order_capture_unproven"),
                ("messages.mute", ["Hide Alerts", "Show Alerts"], "mute_capture_mapping_unproven")
            ] {
                let count = labels.filter { titles.contains($0) }.count
                if operation == "messages.mute" { steps.append(proveMute(prepared, titles: titles)) }
                else { steps.append(.object(["operation": .string(operation), "proven": .bool(false), "reason": .string(count == 1 ? reason : "conversation_menu_target_unavailable"), "control_found": .bool(count == 1)])) }
            }
            steps.append(.object(["operation": .string("messages.set_read_receipts"), "proven": .bool(false), "reason": .string("receipt_details_target_unproven")]))
            for operation in ["messages.recover_chat", "messages.delete_chat_permanently"] {
                steps.append(.object(["operation": .string(operation), "proven": .bool(false), "reason": .string("recently_deleted_exact_target_unproven")]))
            }
            return steps
        } catch let error as MessagesUIFailure { return limited(error.rawValue) }
        catch { return limited(failureReason(error)) }
    }
}

extension MessagesParity {
    static func muteProfile() -> JSON {
        let ledger = report()
        guard ledger["cleanup_state"].string == "verified_clean" else { return .null }
        return ledger["steps"].array.first { $0["operation"].string == "messages.mute" && $0["proven"].bool && ["iMessage", "SMS", "RCS"].contains($0["service"].string) } ?? .null
    }
    static func capturedFields(_ data: Data?, service: String, profile supplied: JSON? = nil) -> [String: Any] {
        let profile = supplied ?? muteProfile()
        guard !profile["property"].string.isEmpty, profile["service"].string == service else { return [:] }
        guard let data else { return profile["missing_is_unmuted"].bool ? ["muted": false] : [:] }
        guard let values = MessageArchive.decode(data) else { return [:] }
        if let value = values[profile["property"].string] as? NSNumber, CFGetTypeID(value) == CFBooleanGetTypeID() {
            return ["muted": value.boolValue == profile["muted_value"].bool]
        }
        return profile["missing_is_unmuted"].bool && values[profile["property"].string] == nil ? ["muted": false] : [:]
    }
    static func prepare(_ command: JSON, path: String) async throws -> Prepared {
        guard command["action"]["op"].string == "messages.mute", muteProfile() != .null else { throw SourceFailure.capabilityUnproven }
        guard case .bool = command["action"]["messages"]["muted"] else { throw SourceFailure.invalid }
        let interactive = try CommandOrigin(command) == .interactive
        let reader = try MessagesReader(path: path)
        let chat = command["action"]["native_key"].string
        guard let conversation = try reader.query("SELECT ROWID AS position,* FROM chat WHERE guid=?", [chat]).first, !(try reader.recoverableChat(chat)) else { throw SourceFailure.conflict }
        let messages = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC", [chat])
        guard let guid = messages.first?["guid"], let row = try reader.message(guid), let observation = try reader.messageObservations(row).first else { throw SourceFailure.capabilityUnproven }
        let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: observation.2))
        let members = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat])
        if !interactive {
            let handles = Set(try MessagesProbes.discoverSelfHandles().handles)
            guard !members.isEmpty, members.allSatisfy({ handles.contains(MessagesProbes.normalize($0["id"])) }), let range = target["text"].string.range(of: #"\[ab-test ([A-Za-z0-9_-]+)\]"#, options: .regularExpression) else { throw SourceFailure.invalid }
            let tag = String(target["text"].string[range]), run = String(target["text"].string[range].dropFirst(9).dropLast())
            let ledger = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")))
            guard ledger["created_chat_key"].string == chat, ledger["created_by_run"].bool, conversation.integer("position") > Int64(ledger["baseline_max_chat_row_id"].number) else { throw SourceFailure.invalid }
            for member in messages {
                guard let item = try reader.message(member["guid"]), let decoded = try reader.messageObservations(item).first, (decoded.2["text"] as? String)?.contains(tag) == true else { throw SourceFailure.invalid }
            }
        }
        guard let current = capturedFields(conversation.blobs["properties"], service: conversation["service_name"])["muted"] as? Bool else { throw SourceFailure.capabilityUnproven }
        let state = JSON.object(["conversation_target": .bool(true), "target": target, "message_guid": .string(guid), "members": .array(messages.map { .string($0["guid"]) }), "before_muted": .bool(current), "properties_digest": .string(BodyDigest.hash(conversation.blobs["properties"] ?? Data()))])
        let prepared = Prepared(nativeState: state, fingerprint: command["request_hash"].string, action: command["action"])
        try await MessagesAccessibility.openMessage(guid)
        try await MessagesAccessibility.validate(prepared)
        return prepared
    }
    static func dispatch(_ prepared: Prepared, path: String) async throws -> JSON {
        guard prepared.action["op"].string == "messages.mute", muteProfile() != .null else { throw SourceFailure.capabilityUnproven }
        let reader = try MessagesReader(path: path), chat = prepared.action["native_key"].string
        guard let row = try reader.query("SELECT properties,service_name FROM chat WHERE guid=?", [chat]).first, row["service_name"] == muteProfile()["service"].string, BodyDigest.hash(row.blobs["properties"] ?? Data()) == prepared.nativeState["properties_digest"].string else { throw SourceFailure.conflict }
        let messages = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC", [chat])
        guard JSON.array(messages.map { .string($0["guid"]) }) == prepared.nativeState["members"] else { throw SourceFailure.conflict }
        let value = prepared.action["messages"]["muted"].bool
        try await mute(prepared, value: value)
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        repeat {
            let row = try reader.query("SELECT properties,service_name FROM chat WHERE guid=?", [chat]).first
            if capturedFields(row?.blobs["properties"], service: row?["service_name"] ?? "")["muted"] as? Bool == value {
                return .object(["source": .string("messages"), "collection": .string("messages.chat"), "native_key": .string(chat), "observed": .bool(true), "disposition": .string("locally_observed")])
            }
            try await Task.sleep(for: .milliseconds(50))
        } while ContinuousClock.now < deadline
        return .object(["source": .string("messages"), "observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string("mute_capture_not_observed")])
    }
}

extension MessagesParity {
    public static func ownedFixture() throws -> (chat: String, handle: String, run: String, baseline: Int64)? {
        let home = PrivateReport.directory.appendingPathComponent("messages-command-cleanup", isDirectory: true)
        guard FileManager.default.fileExists(atPath: home.path) else { return nil }
        let files = try FileManager.default.contentsOfDirectory(at: home, includingPropertiesForKeys: nil).filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let reader = try MessagesReader(path: path), handles = Set(try MessagesProbes.discoverSelfHandles().handles)
        var recoverable: (chat: String, handle: String, run: String, baseline: Int64)?
        for file in files.prefix(2000) {
            guard let data = try? Data(contentsOf: file), let record = try? JSONDecoder().decode(JSON.self, from: data), record["created_by_run"].bool, record["cleanup_state"].string == "verified_clean", !record["created_chat_key"].string.isEmpty else { continue }
            let chat = record["created_chat_key"].string, run = record["run_id"].string
            guard run.range(of: #"^[A-Za-z0-9_-]+$"#, options: .regularExpression) != nil,
                  let row = try reader.query("SELECT ROWID AS position FROM chat WHERE guid=?", [chat]).first,
                  row.integer("position") > Int64(record["baseline_max_chat_row_id"].number) else { continue }
            let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat])
            guard peers.count == 1, let raw = peers.first?["id"], handles.contains(MessagesProbes.normalize(raw)) else { continue }
            let messages = try reader.query("SELECT m.guid,m.ROWID AS position FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=?", [chat])
            var owned = true
            for item in messages {
                guard item.integer("position") > Int64(record["baseline_max_row_id"].number), let row = try reader.message(item["guid"]), let decoded = try reader.messageObservations(row).first,
                      (decoded.2["text"] as? String)?.contains("[ab-test " + run + "]") == true else { owned = false; break }
            }
            if owned {
                let candidate = (chat, MessagesProbes.normalize(raw), run, Int64(record["baseline_max_chat_row_id"].number))
                if try reader.recoverableChat(chat) { if recoverable == nil { recoverable = candidate } }
                else { return candidate }
            }
        }
        return recoverable
    }
}
