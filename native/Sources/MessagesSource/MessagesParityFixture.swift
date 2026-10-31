import Foundation
import SourceSupport

public enum MessagesParityFixture {
    public static func send(text: String, handle: String, chat: String? = nil) async throws -> JSON {
        guard text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\] .+"#, options: .regularExpression) != nil,
              text.utf16.count <= 512, try MessagesProbes.discoverSelfHandles().handles.contains(handle) else { throw SourceFailure.invalid }
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        let baseline = try reader.maximum()
        if let chat {
            let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat])
            guard peers.count == 1, MessagesProbes.normalize(peers[0]["id"]) == handle,
                  let item = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC LIMIT 1", [chat]).first,
                  let row = try reader.message(item["guid"]), let decoded = try reader.messageObservations(row).first else { throw SourceFailure.conflict }
            let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: decoded.2))
            try await sendText(text, guid: item["guid"], target: target)
        } else {
            guard try MessagesSelfTestIsolation.unused([handle], database: MessagesDatabase()) == [handle] else { throw SourceFailure.conflict }
            try await MessagesAccessibility.createGroup(.object(["handles": .array([.string(handle)]), "service": .string("iMessage"), "text": .string(text)]))
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        repeat {
            let rows = try reader.query("SELECT m.guid,m.error,c.guid AS chat_key,c.service_name FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=1 AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(baseline), text, text])
            guard rows.count <= 1 else { break }
            if let row = rows.first {
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["chat_key"]])
                guard row.integer("error") == 0, row["service_name"] == "iMessage", peers.count == 1,
                      MessagesProbes.normalize(peers[0]["id"]) == handle, chat == nil || chat == row["chat_key"] else { break }
                let echoes = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(baseline), row["chat_key"], text, text])
                if echoes.count == 1 {
                    return .object(["native_key": .string(row["guid"]), "chat_key": .string(row["chat_key"]), "observed": .bool(true), "disposition": .string("locally_observed")])
                }
            }
            try await Task.sleep(for: .milliseconds(100))
        } while ContinuousClock.now < deadline
        return .object(["observed": .bool(false), "disposition": .string("indeterminate")])
    }
    @UIActor
    private static func sendText(_ text: String, guid: String, target: JSON) throws {
        try MessagesAccessibility.openMessage(guid, settle: 0.5)
        let prepared = Prepared(nativeState: .object(["target": target]), fingerprint: "owned-self-fixture", action: .object(["native_key": .string(guid)]))
        try MessagesAccessibility.validate(prepared)
        let session = try MessagesAccessibility.uiSession()
        let editors = MessagesVariantUI.find(session.root, identifier: "messageBodyField")
        guard editors.count == 1, MessagesVariantUI.value(editors[0], "AXValue").isEmpty else { throw MessagesUIFailure.draftPresent }
        try MessagesAccessibility.replace(session, text: text)
        try MessagesAccessibility.validate(prepared)
        try session.press(MessagesUISession.enter)
    }
}
