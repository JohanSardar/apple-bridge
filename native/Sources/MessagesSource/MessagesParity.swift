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
            return .object(["operation": .string(operation), "proven": .bool(["messages.pin", "messages.mute", "messages.set_read_receipts"].contains(operation) && step?["proven"].bool == true && ledger["cleanup_state"].string == "verified_clean" && ledger["baseline_preserved"].bool && ["iMessage", "SMS", "RCS"].contains(step?["service"].string ?? "")), "reason": .string(step?["reason"].string ?? "parity_selftest_required")])
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
    static func conversationMenu(_ prepared: Prepared) throws -> (MessagesUISession, [AXUIElement]) {
        try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string, settle: 0.5)
        let session = try MessagesAccessibility.uiSession()
        var state = prepared.nativeState.object
        state["conversation_target"] = .bool(false)
        try MessagesAccessibility.validate(Prepared(nativeState: .object(state), fingerprint: prepared.fingerprint, action: prepared.action))
        let menus = MessagesVariantUI.find(session.root, role: kAXMenuBarItemRole, name: "Conversation")
        guard menus.count == 1 else { throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(menus[0])
        return (session, MessagesVariantUI.find(menus[0], role: kAXMenuItemRole))
    }
    @UIActor
    static func pin(_ prepared: Prepared, value: Bool) throws {
        let (session, items) = try conversationMenu(prepared)
        let desired = value ? ["Pin", "Pin Conversation"] : ["Unpin", "Unpin Conversation"]
        let opposite = value ? ["Unpin", "Unpin Conversation"] : ["Pin", "Pin Conversation"]
        let targets = items.filter { desired.contains(MessagesAccessibility.value($0, kAXTitleAttribute)) }
        if targets.isEmpty, items.filter({ opposite.contains(MessagesAccessibility.value($0, kAXTitleAttribute)) }).count == 1 { try dismiss(session); return }
        guard targets.count == 1 else { try? dismiss(session); throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(targets[0])
    }
    @UIActor
    static func provePin(_ prepared: Prepared) -> JSON {
        var changed = false
        var before: [String] = []
        do {
            before = try MessagesPins.read()
            let chat = prepared.action["native_key"].string
            let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
            guard let row = try MessagesReader(path: path).query("SELECT chat_identifier FROM chat WHERE guid=?", [chat]).first,
                  !row["chat_identifier"].isEmpty, !before.contains(row["chat_identifier"]), before.count < 9 else { throw SourceFailure.capabilityUnproven }
            changed = true
            try pin(prepared, value: true)
            let deadline = Date().addingTimeInterval(5)
            var after = try MessagesPins.read()
            while !after.contains(row["chat_identifier"]), Date() < deadline {
                RunLoop.current.run(until: Date().addingTimeInterval(0.1))
                after = try MessagesPins.read()
            }
            guard after.filter({ $0 != row["chat_identifier"] }) == before, after.contains(row["chat_identifier"]) else { throw SourceFailure.conflict }
            try pin(prepared, value: false)
            let restoreDeadline = Date().addingTimeInterval(5)
            while try MessagesPins.read() != before, Date() < restoreDeadline { RunLoop.current.run(until: Date().addingTimeInterval(0.1)) }
            guard try MessagesPins.read() == before else { throw SourceFailure.conflict }
            changed = false
            return .object(["operation": .string("messages.pin"), "proven": .bool(true), "reason": .string("native_pin_order_roundtrip"), "service": prepared.nativeState["service"]])
        } catch {
            if changed { try? pin(prepared, value: false) }
            return .object(["operation": .string("messages.pin"), "proven": .bool(false), "reason": .string("pin_roundtrip_" + failureReason(error))])
        }
    }
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
            var steps = [provePin(prepared), proveMute(prepared, titles: titles)]
            steps.append(proveReceipts(prepared))
            steps += proveDeleted(prepared)
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
        let operation = command["action"]["op"].string
        guard coverage().contains(where: { $0["operation"].string == operation && $0["proven"].bool }) else { throw SourceFailure.capabilityUnproven }
        let args = command["action"]["messages"]
        switch operation {
        case "messages.mute": guard case .bool = args["muted"] else { throw SourceFailure.invalid }
        case "messages.pin": _ = try MessagesPins.requestedIndex(args["pinned_index"])
        case "messages.set_read_receipts": guard ["on", "off"].contains(args["read_receipts"].string) else { throw SourceFailure.unsupported }
        default: throw SourceFailure.capabilityUnproven
        }
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
        var fields: [String: JSON] = ["conversation_target": .bool(operation == "messages.mute"), "target": target, "message_guid": .string(guid), "members": .array(messages.map { .string($0["guid"]) }), "properties_digest": .string(BodyDigest.hash(conversation.blobs["properties"] ?? Data())), "service": .string(conversation["service_name"])]
        if operation == "messages.mute" {
            guard capturedFields(conversation.blobs["properties"], service: conversation["service_name"])["muted"] is Bool else { throw SourceFailure.capabilityUnproven }
        }
        if operation == "messages.pin" {
            let pins = try MessagesPins.read(), identifier = conversation["chat_identifier"]
            let index = Int(args["pinned_index"].number)
            guard index == -1 || pins.firstIndex(of: identifier) == index || (!pins.contains(identifier) && index == pins.count && pins.count < 9) else { throw SourceFailure.unsupported }
            fields["pins"] = .array(pins.map(JSON.string))
            fields["chat_identifier"] = .string(identifier)
        }
        let state = JSON.object(fields)
        let prepared = Prepared(nativeState: state, fingerprint: command["request_hash"].string, action: command["action"])
        try await MessagesAccessibility.openMessage(guid)
        try await MessagesAccessibility.validate(prepared)
        return prepared
    }
    static func dispatch(_ prepared: Prepared, path: String) async throws -> JSON {
        let operation = prepared.action["op"].string
        guard coverage().contains(where: { $0["operation"].string == operation && $0["proven"].bool }) else { throw SourceFailure.capabilityUnproven }
        let reader = try MessagesReader(path: path), chat = prepared.action["native_key"].string
        guard let row = try reader.query("SELECT properties,service_name FROM chat WHERE guid=?", [chat]).first, row["service_name"] == prepared.nativeState["service"].string, BodyDigest.hash(row.blobs["properties"] ?? Data()) == prepared.nativeState["properties_digest"].string else { throw SourceFailure.conflict }
        let messages = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC", [chat])
        guard JSON.array(messages.map { .string($0["guid"]) }) == prepared.nativeState["members"] else { throw SourceFailure.conflict }
        let args = prepared.action["messages"]
        switch operation {
        case "messages.mute": try await mute(prepared, value: args["muted"].bool)
        case "messages.pin":
            guard JSON.array(try MessagesPins.read().map(JSON.string)) == prepared.nativeState["pins"] else { throw SourceFailure.conflict }
            try await pin(prepared, value: args["pinned_index"].number >= 0)
        case "messages.set_read_receipts": try await receipts(prepared, value: args["read_receipts"].string == "on")
        default: throw SourceFailure.capabilityUnproven
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        repeat {
            let row = try reader.query("SELECT properties,service_name FROM chat WHERE guid=?", [chat]).first
            let observed: Bool
            switch operation {
            case "messages.mute": observed = capturedFields(row?.blobs["properties"], service: row?["service_name"] ?? "")["muted"] as? Bool == args["muted"].bool
            case "messages.pin":
                let pins = try MessagesPins.read(), index = pins.firstIndex(of: prepared.nativeState["chat_identifier"].string)
                observed = args["pinned_index"].number < 0 ? index == nil : index == Int(args["pinned_index"].number)
            case "messages.set_read_receipts": observed = MessageRich.chatProperties(row?.blobs["properties"])["read_receipts"] as? String == args["read_receipts"].string
            default: observed = false
            }
            if observed {
                return .object(["source": .string("messages"), "collection": .string("messages.chat"), "native_key": .string(chat), "observed": .bool(true), "disposition": .string("locally_observed")])
            }
            try await Task.sleep(for: .milliseconds(50))
        } while ContinuousClock.now < deadline
        return .object(["source": .string("messages"), "observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string("conversation_setting_not_observed")])
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
