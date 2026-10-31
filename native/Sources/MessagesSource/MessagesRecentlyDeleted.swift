import Foundation
import AppKit
import ApplicationServices
import Decoders
import SourceSupport

extension MessagesParity {
    static func prepareDeleted(_ command: JSON, path: String) async throws -> Prepared {
        let reader = try MessagesReader(path: path), chat = command["action"]["native_key"].string
        guard try reader.recoverableChat(chat), let row = try reader.query("SELECT ROWID AS position,* FROM chat WHERE guid=?", [chat]).first else { throw SourceFailure.conflict }
        let members = try deletedMembers(chat, reader: reader)
        guard let guid = members.last, let message = try reader.message(guid), let observation = try reader.messageObservations(message).first else { throw SourceFailure.conflict }
        let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: observation.2))
        let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { MessagesProbes.normalize($0["id"]) }
        let own = Set(try MessagesProbes.discoverSelfHandles().handles)
        guard peers.count == 1, own.contains(peers[0]) else { throw SourceFailure.invalid }
        let interactive = try CommandOrigin(command) == .interactive
        if !interactive {
            guard let range = target["text"].string.range(of: #"\[ab-test ([A-Za-z0-9_-]+)\]"#, options: .regularExpression) else { throw SourceFailure.invalid }
            let tag = String(target["text"].string[range]), run = String(tag.dropFirst(9).dropLast())
            let ledger = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")))
            guard ledger["run_id"].string == run, ledger["created_by_run"].bool, ledger["created_chat_key"].string == chat,
                  row.integer("position") > Int64(ledger["baseline_max_chat_row_id"].number) else { throw SourceFailure.invalid }
            for member in members {
                guard let message = try reader.message(member), message.integer("row_position") > Int64(ledger["baseline_max_row_id"].number), let decoded = try reader.messageObservations(message).first,
                      (decoded.2["text"] as? String)?.contains(tag) == true else { throw SourceFailure.invalid }
            }
        }
        let candidates = try reader.query("SELECT DISTINCT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id JOIN chat_recoverable_message_join r ON r.chat_id=c.ROWID WHERE c.guid<>?", [chat])
        guard !candidates.contains(where: { peers.contains(MessagesProbes.normalize($0["id"])) }) else { throw SourceFailure.capabilityUnproven }
        let prepared = Prepared(nativeState: .object(["deleted_target": .bool(true), "target": target, "handles": .array(peers.map(JSON.string)), "members": .array(members.map(JSON.string)), "service": .string(row["service_name"]), "properties_digest": .string(BodyDigest.hash(row.blobs["properties"] ?? Data()))]), fingerprint: command["request_hash"].string, action: command["action"])
        try await validate(prepared)
        return prepared
    }
    @UIActor
    static func validate(_ prepared: Prepared) throws {
        if prepared.nativeState["deleted_target"].bool { _ = try openDeleted(prepared) }
        else { try MessagesAccessibility.validate(prepared) }
    }
    static func dispatchDeleted(_ prepared: Prepared, path: String) async throws -> JSON {
        let reader = try MessagesReader(path: path), chat = prepared.action["native_key"].string
        guard try reader.recoverableChat(chat), JSON.array(try deletedMembers(chat, reader: reader).map(JSON.string)) == prepared.nativeState["members"],
              let row = try reader.query("SELECT properties FROM chat WHERE guid=?", [chat]).first,
              BodyDigest.hash(row.blobs["properties"] ?? Data()) == prepared.nativeState["properties_digest"].string else { throw SourceFailure.conflict }
        let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { MessagesProbes.normalize($0["id"]) }
        guard JSON.array(peers.map(JSON.string)) == prepared.nativeState["handles"] else { throw SourceFailure.conflict }
        let permanently = prepared.action["op"].string == "messages.delete_chat_permanently"
        try await deletedAction(prepared, permanently: permanently)
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        repeat {
            let observed: Bool
            if permanently { observed = try reader.query("SELECT 1 FROM chat WHERE guid=?", [chat]).isEmpty }
            else {
                let active = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? ORDER BY m.guid", [chat]).map { $0["guid"] }
                observed = try !reader.recoverableChat(chat) && deletedMembers(chat, reader: reader).isEmpty && JSON.array(active.map(JSON.string)) == prepared.nativeState["members"]
            }
            if observed { return .object(["source": .string("messages"), "collection": .string("messages.chat"), "native_key": .string(chat), "observed": .bool(true), "absent": .bool(permanently), "disposition": .string("locally_observed")]) }
            try await Task.sleep(for: .milliseconds(50))
        } while ContinuousClock.now < deadline
        return .object(["source": .string("messages"), "observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string("recently_deleted_change_not_observed")])
    }
    static func deletedMembers(_ chat: String, reader: MessagesReader) throws -> [String] {
        try reader.query("SELECT m.guid FROM message m JOIN chat_recoverable_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? ORDER BY m.guid", [chat]).map { $0["guid"] }
    }
    @UIActor
    static func deletedTarget(_ prepared: Prepared, session: MessagesUISession) throws -> AXUIElement {
        let chat = prepared.action["native_key"].string
        let handles = Set(prepared.nativeState["handles"].array.map(\.string))
        let expected = prepared.nativeState["target"]["text"].string
        var budget = 4000
        let matches = MessagesAccessibility.matching(session.root, budget: &budget) { element in
            guard MessagesAccessibility.value(element, kAXRoleAttribute) == kAXRowRole else { return false }
            var nested = 100
            let found = MessagesAccessibility.matching(element, budget: &nested) { node in
                [kAXIdentifierAttribute, kAXValueAttribute, kAXDescriptionAttribute, kAXTitleAttribute].contains { attribute in
                    let value = MessagesAccessibility.value(node, attribute)
                    return value == chat || (!expected.isEmpty && value == expected) || (!value.isEmpty && handles.contains(MessagesProbes.normalize(value)))
                }
            }
            return nested > 0 && !found.isEmpty
        }
        guard budget > 0, matches.count == 1 else { throw MessagesUIFailure.targetUnavailable }
        return matches[0]
    }
    @UIActor
    static func openDeleted(_ prepared: Prepared) throws -> MessagesUISession {
        MessagesUISession.activate()
        let session = try MessagesAccessibility.uiSession()
        let menus = MessagesVariantUI.find(session.root, role: kAXMenuBarItemRole, name: "View")
        guard menus.count == 1 else { throw MessagesUIFailure.menuUnavailable }
        try MessagesVariantUI.press(menus[0])
        let items = MessagesVariantUI.find(menus[0], role: kAXMenuItemRole).filter { ["Recently Deleted", "Show Recently Deleted"].contains(MessagesAccessibility.value($0, kAXTitleAttribute)) }
        guard items.count == 1 else { try? dismiss(session); throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(items[0])
        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        let row = try deletedTarget(prepared, session: session)
        try session.fence()
        guard AXUIElementSetAttributeValue(row, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else { throw MessagesUIFailure.targetUnavailable }
        var selected: CFTypeRef?
        guard AXUIElementCopyAttributeValue(row, kAXSelectedAttribute as CFString, &selected) == .success, (selected as? NSNumber)?.boolValue == true else { throw MessagesUIFailure.targetUnavailable }
        var budget = 4000
        var table = row
        var tableFound = false
        for _ in 0..<6 {
            if [kAXTableRole, kAXOutlineRole].contains(MessagesAccessibility.value(table, kAXRoleAttribute)) { tableFound = true; break }
            var parent: CFTypeRef?
            guard AXUIElementCopyAttributeValue(table, kAXParentAttribute as CFString, &parent) == .success, let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
            table = unsafeDowncast(parent, to: AXUIElement.self)
        }
        guard tableFound else { throw MessagesUIFailure.targetUnavailable }
        let selections = MessagesAccessibility.matching(table, budget: &budget) { node in
            guard MessagesAccessibility.value(node, kAXRoleAttribute) == kAXRowRole else { return false }
            var value: CFTypeRef?
            return AXUIElementCopyAttributeValue(node, kAXSelectedAttribute as CFString, &value) == .success && (value as? NSNumber)?.boolValue == true
        }
        guard budget > 0, selections.count == 1, CFEqual(selections[0], row) else { throw SourceFailure.conflict }
        return session
    }
    @UIActor
    static func deletedAction(_ prepared: Prepared, permanently: Bool) throws {
        let session = try openDeleted(prepared)
        defer { try? dismiss(session) }
        _ = try deletedTarget(prepared, session: session)
        try MessagesAccessibility.press(session, title: permanently ? "Delete" : "Recover")
        let confirmation = permanently ? ["Delete Messages", "Delete"] : ["Recover Messages", "Recover"]
        let sheets = MessagesVariantUI.find(session.root, role: kAXSheetRole)
        guard sheets.count == 1 else { throw MessagesUIFailure.menuUnavailable }
        let buttons = confirmation.flatMap { MessagesVariantUI.find(sheets[0], role: kAXButtonRole, name: $0) }
        guard buttons.count == 1 else { throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(buttons[0])
    }
    @UIActor
    static func proveDeleted(_ prepared: Prepared) -> [JSON] {
        let operations = ["messages.recover_chat", "messages.delete_chat_permanently"]
        var recovered = false
        do {
            let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
            let reader = try MessagesReader(path: path), chat = prepared.action["native_key"].string
            let text = prepared.nativeState["target"]["text"].string
            guard let range = text.range(of: #"\[ab-test ([A-Za-z0-9_-]+)\]"#, options: .regularExpression),
                  let position = try reader.query("SELECT ROWID AS position FROM chat WHERE guid=?", [chat]).first else { throw SourceFailure.invalid }
            let run = String(text[range].dropFirst(9).dropLast())
            let ledger = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")))
            if ledger["existing_self_chat"].bool {
                return try proveOwnedDeletedMessages(prepared, ledger: ledger, reader: reader, tag: String(text[range]))
            }
            guard ledger["run_id"].string == run, ledger["created_by_run"].bool, ledger["created_chat_key"].string == chat,
                  position.integer("position") > Int64(ledger["baseline_max_chat_row_id"].number) else { throw SourceFailure.invalid }
            let handles = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { MessagesProbes.normalize($0["id"]) }
            guard handles.count == 1, Set(try MessagesProbes.discoverSelfHandles().handles).contains(handles[0]) else { throw SourceFailure.invalid }
            let others = try reader.query("SELECT DISTINCT c.guid FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id JOIN chat_recoverable_message_join r ON r.chat_id=c.ROWID WHERE c.guid<>?", [chat])
            for other in others {
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [other["guid"]])
                guard !peers.contains(where: { handles.contains(MessagesProbes.normalize($0["id"])) }) else { throw SourceFailure.capabilityUnproven }
            }
            var state = prepared.nativeState.object
            state["handles"] = .array(handles.map(JSON.string))
            let target = Prepared(nativeState: .object(state), fingerprint: prepared.fingerprint, action: .object(["op": .string("messages.delete_chat"), "native_key": .string(chat)]))
            func activeMembers() throws -> [String] {
                try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.guid", [chat]).map { $0["guid"] }
            }
            let members = try activeMembers()
            guard !members.isEmpty else { throw SourceFailure.conflict }
            for member in members {
                guard let row = try reader.message(member), row.integer("row_position") > Int64(ledger["baseline_max_row_id"].number), let decoded = try reader.messageObservations(row).first,
                      (decoded.2["text"] as? String) == text else { throw SourceFailure.invalid }
            }
            func deleted() throws -> Bool { try reader.recoverableChat(chat) && deletedMembers(chat, reader: reader) == members }
            func wait(_ condition: () throws -> Bool) throws {
                let deadline = Date().addingTimeInterval(5)
                repeat {
                    if try condition() { return }
                    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
                } while Date() < deadline
                throw SourceFailure.conflict
            }
            try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string, settle: 0.5)
            _ = try MessagesAccessibility.perform(target)
            try wait(deleted)
            try deletedAction(target, permanently: false)
            try wait { try !reader.recoverableChat(chat) && deletedMembers(chat, reader: reader).isEmpty && activeMembers() == members }
            recovered = true
            try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string, settle: 0.5)
            _ = try MessagesAccessibility.perform(target)
            try wait(deleted)
            try deletedAction(target, permanently: true)
            try wait {
                try reader.query("SELECT 1 FROM chat WHERE guid=?", [chat]).isEmpty && reader.query("SELECT 1 FROM chat_recoverable_message_join j JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? UNION ALL SELECT 1 FROM chat_message_join j JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=?", [chat, chat]).isEmpty
            }
            return operations.map { .object(["operation": .string($0), "proven": .bool(true), "reason": .string("owned_conversation_native_recovery_and_purge"), "service": prepared.nativeState["service"]]) }
        } catch {
            return operations.map { operation in .object(["operation": .string(operation), "proven": .bool(operation == "messages.recover_chat" && recovered), "reason": .string("recently_deleted_" + failureReason(error)), "service": prepared.nativeState["service"]]) }
        }
    }
}
