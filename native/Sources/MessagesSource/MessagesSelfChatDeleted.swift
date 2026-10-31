import Foundation
import AppKit
import SourceSupport

extension MessagesParity {
    @UIActor
    static func proveOwnedDeletedMessages(_ prepared: Prepared, ledger: JSON, reader: MessagesReader, tag: String) throws -> [JSON] {
        let chat = prepared.action["native_key"].string
        let run = String(tag.dropFirst(9).dropLast())
        guard ledger["run_id"].string == run, ledger["existing_self_chat"].bool, !ledger["created_by_run"].bool,
              ledger["created_chat_key"].string == chat else { throw SourceFailure.invalid }
        let baseline = Int64(ledger["baseline_max_row_id"].number)
        func owned() throws -> [String] {
            let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { $0["id"] }
            guard MessagesSelfTestIsolation.isSelfChat(peers, own: try MessagesSelfTestIsolation.accountHandles(database: MessagesDatabase())) else { throw SourceFailure.invalid }
            let rows = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.ROWID>? ORDER BY m.guid", [chat, String(baseline)])
            var ids: [String] = []
            for item in rows {
                guard let message = try reader.message(item["guid"]), let observation = try reader.messageObservations(message).first,
                      let text = observation.2["text"] as? String,
                      MessagesSelfTestIsolation.ownsMessage(rowID: message.integer("row_position"), text: text, tag: tag, baseline: baseline) else { continue }
                let memberships = try reader.query("SELECT c.guid FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", [item["guid"]])
                guard memberships.count == 1, memberships[0]["guid"] == chat else { throw SourceFailure.conflict }
                ids.append(item["guid"])
            }
            return ids
        }
        let ids = try owned()
        guard !ids.isEmpty else { throw SourceFailure.conflict }
        let existingDeleted = try deletedMembers(chat, reader: reader)
        guard existingDeleted.isEmpty else {
            return ["messages.recover_chat", "messages.delete_chat_permanently"].map {
                .object(["operation": .string($0), "proven": .bool(false), "reason": .string("existing_deleted_messages_protected"), "protected_count": .number(Double(existingDeleted.count)), "service": prepared.nativeState["service"]])
            }
        }
        let baselineDigest = try MessagesSelfTestBaseline.digest(chat, through: baseline)
        var state = prepared.nativeState.object
        state["handles"] = .array(try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { .string(MessagesProbes.normalize($0["id"])) })
        let deletedTarget = Prepared(nativeState: .object(state), fingerprint: prepared.fingerprint, action: prepared.action)
        func wait(_ condition: () throws -> Bool) throws {
            let deadline = Date().addingTimeInterval(5)
            repeat {
                if try condition() { return }
                RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            } while Date() < deadline
            throw SourceFailure.conflict
        }
        func deleteOwned() throws {
            guard try owned() == ids, try deletedMembers(chat, reader: reader).isEmpty else { throw SourceFailure.conflict }
            for guid in ids {
                guard let row = try reader.message(guid), let observation = try reader.messageObservations(row).first,
                      let text = observation.2["text"] as? String,
                      MessagesSelfTestIsolation.ownsMessage(rowID: row.integer("row_position"), text: text, tag: tag, baseline: baseline) else { throw SourceFailure.invalid }
                let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: observation.2))
                let command = Prepared(nativeState: .object(["target": target, "cleanup_authorized": .bool(true)]), fingerprint: prepared.fingerprint, action: .object(["op": .string("messages.delete_message"), "native_key": .string(guid)]))
                try MessagesAccessibility.openMessage(guid, settle: 0.5)
                try MessagesAccessibility.validate(command)
                _ = try MessagesAccessibility.perform(command)
            }
            try wait { try deletedMembers(chat, reader: reader) == ids }
            guard try MessagesSelfTestBaseline.digest(chat, through: baseline) == baselineDigest else { throw SourceFailure.conflict }
        }
        try deleteOwned()
        guard try deletedMembers(chat, reader: reader) == ids, try owned() == ids else { throw SourceFailure.conflict }
        try deletedAction(deletedTarget, permanently: false)
        try wait { try deletedMembers(chat, reader: reader).isEmpty && owned() == ids }
        try deleteOwned()
        guard try deletedMembers(chat, reader: reader) == ids, try owned() == ids else { throw SourceFailure.conflict }
        try deletedAction(deletedTarget, permanently: true)
        try wait {
            try deletedMembers(chat, reader: reader).isEmpty && owned().isEmpty && !reader.query("SELECT 1 FROM chat WHERE guid=?", [chat]).isEmpty
        }
        guard try MessagesSelfTestBaseline.digest(chat, through: baseline) == baselineDigest else { throw SourceFailure.conflict }
        return ["messages.recover_chat", "messages.delete_chat_permanently"].map {
            .object(["operation": .string($0), "proven": .bool(true), "reason": .string("owned_messages_native_recovery_and_purge"), "service": prepared.nativeState["service"]])
        }
    }
}
