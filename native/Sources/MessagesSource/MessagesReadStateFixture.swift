import Foundation
import SourceSupport

public enum MessagesReadStateFixture {
    public static func authorizes(_ registration: JSON, chat: String, rowID: Int64, text: String) -> Bool {
        let run = registration["run_id"].string
        guard run.range(of: #"^[A-Za-z0-9_-]+$"#, options: .regularExpression) != nil,
              ["MSG-SELFTEST", "MSG-READSTATE-REAL"].contains(registration["scenario"].string),
              let baseline = Int64(exactly: registration["baseline_max_row_id"].number), baseline > 0,
              rowID > baseline, text.hasPrefix("[ab-test " + run + "] "), !chat.isEmpty else { return false }
        return registration["created_chat_key"].string.isEmpty || registration["created_chat_key"].string == chat
    }

    static func validate(chat: String, target: SQLiteRow, reader: MessagesReader) throws {
        guard let decoded = try reader.messageObservations(target).first,
              let text = decoded.2["text"] as? String,
              let range = text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) else { throw SourceFailure.invalid }
        let tag = String(text[range])
        let run = String(tag.dropFirst(9).dropLast())
        let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")
        let registration = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: ledger))
        guard authorizes(registration, chat: chat, rowID: target.integer("row_position"), text: text) else { throw SourceFailure.invalid }
        let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat]).map { $0["id"] }
        guard MessagesSelfTestIsolation.isSelfChat(peers, own: try MessagesSelfTestIsolation.accountHandles(database: MessagesDatabase())) else { throw SourceFailure.invalid }
        let baseline = String(Int64(registration["baseline_max_row_id"].number))
        let foreignUnread = try reader.query("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.ROWID<=? AND m.is_from_me=0 AND m.is_read=0 AND COALESCE(m.associated_message_type,0)=0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) LIMIT 1", [chat, baseline])
        let foreignNew = try reader.query("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.ROWID>? AND NOT (instr(COALESCE(m.text,''),?)>0 OR COALESCE(instr(m.attributedBody,CAST(? AS BLOB)),0)>0) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) LIMIT 1", [chat, baseline, tag, tag])
        guard foreignUnread.isEmpty, foreignNew.isEmpty else { throw SourceFailure.conflict }
    }
}
