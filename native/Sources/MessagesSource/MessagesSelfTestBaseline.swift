import Foundation
import ApplicationServices
import SourceSupport
import Decoders

public enum MessagesSelfTestBaselineFailure: Error, CustomStringConvertible {
    case unreadMessages

    public var description: String {
        "The self conversation contains unread existing messages. This check cannot open it without changing their read state. Read those messages in Messages before rerunning the check."
    }
}

public struct MessagesContentBaseline: Sendable {
    public let chat: String
    private let lastMessageROWID: Int64
    private let messagesDigest: String
    private let preferencesDigest: String
    private let pins: [String]

    public static func capture(_ chat: String) throws -> MessagesContentBaseline {
        let reader = try MessagesSelfTestBaseline.reader()
        let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { $0["id"] }
        guard MessagesSelfTestIsolation.isSelfChat(peers, own: try MessagesSelfTestIsolation.accountHandles(database: MessagesDatabase())) else { throw SourceFailure.invalid }
        guard try reader.query("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.is_from_me=0 AND m.is_read=0 AND COALESCE(m.associated_message_type,0)=0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) LIMIT 1", [chat]).isEmpty else { throw MessagesSelfTestBaselineFailure.unreadMessages }
        let maximum = try reader.maximum()
        return MessagesContentBaseline(chat: chat, lastMessageROWID: maximum, messagesDigest: try MessagesSelfTestBaseline.digest(chat, through: maximum), preferencesDigest: try preferenceDigest(MessagesBaselineProperties.properties(chat)), pins: try MessagesPins.read())
    }

    public static func preferenceDigest(_ properties: [String: Any]) throws -> String {
        func canonical(_ value: Any) -> Any {
            if let date = value as? Date { return ["date": date.timeIntervalSinceReferenceDate] }
            if let bytes = value as? Data { return ["data_digest": BodyDigest.hash(bytes)] }
            if let fields = value as? [String: Any] { return fields.mapValues(canonical) }
            if let items = value as? [Any] { return items.map(canonical) }
            return value
        }
        let transient = Set(["lastSeenMessageGuid", "LSMD", "numberOfTimesRespondedtoThread"])
        let preferences = properties.filter { !transient.contains($0.key) }.mapValues(canonical)
        return BodyDigest.hash(try JSONSerialization.data(withJSONObject: preferences, options: [.sortedKeys]))
    }

    public func restoreAndVerify() throws -> Bool {
        try MessagesSelfTestBaseline.digest(chat, through: lastMessageROWID) == messagesDigest && Self.preferenceDigest(MessagesBaselineProperties.properties(chat)) == preferencesDigest && MessagesPins.read() == pins
    }
}

enum MessagesSelfTestBaseline {    static func reader() throws -> MessagesReader {
        try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
    }

    static func target(_ chat: String) throws -> Prepared {
        let reader = try reader()
        let members = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { $0["id"] }
        guard MessagesSelfTestIsolation.isSelfChat(members, own: try MessagesSelfTestIsolation.accountHandles(database: MessagesDatabase())),
              let row = try reader.query("SELECT c.service_name,m.guid FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE c.guid=? AND m.is_from_me=1 AND COALESCE(m.associated_message_type,0)=0 AND m.item_type=0 AND COALESCE(m.date_retracted,0)=0 AND length(m.text)>0 AND NOT EXISTS(SELECT 1 FROM message_attachment_join a WHERE a.message_id=m.ROWID) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC LIMIT 1", [chat]).first,
              let message = try reader.message(row["guid"]), let observation = try reader.messageObservations(message).first else { throw SourceFailure.conflict }
        let data = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: observation.2))
        return Prepared(nativeState: .object(["conversation_target": .bool(true), "service": .string(row["service_name"]), "message_guid": .string(row["guid"]), "target": data]), fingerprint: "self-chat-baseline", action: .object(["native_key": .string(chat)]))
    }

    static func digest(_ chat: String, through baseline: Int64) throws -> String {
        let reader = try reader()
        let rows = try reader.query("SELECT m.*,j.chat_id AS baseline_chat_id,r.delete_date AS baseline_delete_date FROM message m JOIN (SELECT chat_id,message_id FROM chat_message_join UNION SELECT chat_id,message_id FROM chat_recoverable_message_join) j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id LEFT JOIN chat_recoverable_message_join r ON r.chat_id=j.chat_id AND r.message_id=m.ROWID WHERE c.guid=? AND m.ROWID<=? ORDER BY m.ROWID", [chat, String(baseline)])
        let values = rows.map { row -> [String: String] in
            var fields = row.values
            if row.integer("associated_message_type") != 0 { fields.removeValue(forKey: "is_read"); fields.removeValue(forKey: "date_read") }
            for (key, bytes) in row.blobs { fields[key] = BodyDigest.hash(bytes) }
            return fields
        }
        return BodyDigest.hash(try JSONSerialization.data(withJSONObject: values, options: [.sortedKeys]))
    }

}
