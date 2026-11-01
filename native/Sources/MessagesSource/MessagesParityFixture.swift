import Foundation
import ApplicationServices
import SourceSupport

public enum MessagesParityFixture {
    public static func recipientMatches(_ description: String, handle: String) -> Bool {
        let expected = MessagesProbes.normalize(handle)
        guard !expected.isEmpty else { return false }
        if expected.contains("@") {
            let pattern = #"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+"#
            guard let expression = try? NSRegularExpression(pattern: pattern) else { return false }
            let range = NSRange(description.startIndex..., in: description)
            let addresses = expression.matches(in: description, range: range).compactMap { Range($0.range, in: description).map { String(description[$0]).lowercased() } }
            return addresses == [expected]
        }
        return description.filter(\.isNumber) == expected.filter(\.isNumber)
    }
    @UIActor
    static func sendExisting(_ text: String, chat: String) throws {
        let baseline = try MessagesSelfTestBaseline.target(chat)
        var state = baseline.nativeState.object
        state["conversation_target"] = .bool(false)
        let target = Prepared(nativeState: .object(state), fingerprint: baseline.fingerprint, action: baseline.action)
        try MessagesAccessibility.openMessage(target.nativeState["message_guid"].string, settle: 0.5, resetEditor: false)
        try MessagesAccessibility.validateConversation(target)
        let session = try MessagesAccessibility.uiSession()
        let fields = MessagesVariantUI.find(session.root, identifier: "messageBodyField")
        guard fields.count == 1, MessagesVariantUI.value(fields[0], kAXValueAttribute).isEmpty else { throw MessagesUIFailure.draftPresent }
        guard MessagesVariantUI.value(fields[0], kAXPlaceholderValueAttribute).contains("iMessage") else { throw MessagesUIFailure.wrongService }
        try session.type(text, into: fields[0], value: { MessagesVariantUI.value($0, kAXValueAttribute) })
        try MessagesAccessibility.validateConversation(target)
        guard session.focused(fields[0]), MessagesVariantUI.value(fields[0], kAXValueAttribute) == text else { throw MessagesUIFailure.draftPresent }
        try session.press(MessagesUISession.enter)
    }
    public static func send(text: String, handle: String, existingChat: String = "") async throws -> JSON {
        guard text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\] .+"#, options: .regularExpression) != nil,
              text.utf16.count <= 512, try MessagesProbes.discoverSelfHandles().handles.contains(handle) else { throw SourceFailure.invalid }
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        let baseline = try reader.maximum()
        if existingChat.isEmpty {
            guard try MessagesSelfTestIsolation.unused([handle], database: MessagesDatabase()) == [handle] else { throw SourceFailure.conflict }
        } else {
            let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [existingChat]).map { $0["id"] }
            guard MessagesSelfTestIsolation.isSelfChat(peers, own: [handle]),
                  MessagesSelfTestIsolation.isSelfChat([handle], own: try MessagesSelfTestIsolation.accountHandles(database: MessagesDatabase())) else { throw SourceFailure.conflict }
        }
        if existingChat.isEmpty {
            try await MessagesAccessibility.createGroup(.object(["handles": .array([.string(handle)]), "service": .string("iMessage"), "text": .string(text)]))
        } else {
            try await sendExisting(text, chat: existingChat)
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        repeat {
            let rows = try reader.query("SELECT m.guid,m.error,c.guid AS chat_key,c.service_name FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=1 AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(baseline), text, text])
            guard rows.count <= 1 else { break }
            if let row = rows.first {
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["chat_key"]])
                guard (existingChat.isEmpty || row["chat_key"] == existingChat), row.integer("error") == 0, row["service_name"] == "iMessage", peers.count == 1,
                      MessagesProbes.normalize(peers[0]["id"]) == handle else { break }
                let echoes = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(baseline), row["chat_key"], text, text])
                if echoes.count == 1 {
                    return .object(["native_key": .string(row["guid"]), "chat_key": .string(row["chat_key"]), "observed": .bool(true), "disposition": .string("locally_observed")])
                }
            }
            try await Task.sleep(for: .milliseconds(100))
        } while ContinuousClock.now < deadline
        return .object(["observed": .bool(false), "disposition": .string("indeterminate")])
    }
}
