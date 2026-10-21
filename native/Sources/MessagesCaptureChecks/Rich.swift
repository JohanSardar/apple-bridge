import Foundation
import CryptoKit
import MessagesSource
import SourceSupport

struct RichCaptureChecks {
    static func run() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("w2-rich-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let path = root.appendingPathComponent("chat.db").path
        defer {
            let digest = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
            let baseline = NSHomeDirectory() + "/Library/Application Support/AppleBridge/messages/" + digest + ".sqlite"
            for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: baseline + suffix) }
            try? FileManager.default.removeItem(at: root)
        }
        func fixture(_ name: String) throws -> String {
            try Data(contentsOf: URL(fileURLWithPath: "lab/fixtures/messages/" + name)).map { String(format: "%02x", $0) }.joined()
        }
        let db = try SyntheticDatabase(path)
        try db.execute("PRAGMA journal_mode=WAL; CREATE TABLE message(guid TEXT,text TEXT,balloon_bundle_id TEXT,payload_data BLOB,attributedBody BLOB,is_audio_message INTEGER DEFAULT 0,date_played INTEGER DEFAULT 0,is_expirable INTEGER DEFAULT 0,schedule_type INTEGER DEFAULT 0,schedule_state INTEGER DEFAULT 0,date INTEGER DEFAULT 0,subject TEXT,was_delivered_quietly INTEGER DEFAULT 0,did_notify_recipient INTEGER DEFAULT 0,is_spam INTEGER DEFAULT 0,has_unseen_mention INTEGER DEFAULT 0,is_stewie INTEGER DEFAULT 0,is_sos INTEGER DEFAULT 0,associated_message_guid TEXT,associated_message_type INTEGER DEFAULT 0); CREATE TABLE chat(guid TEXT,is_filtered INTEGER,properties BLOB); CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER); CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER); CREATE TABLE attachment(guid TEXT,filename TEXT,is_sticker INTEGER,sticker_user_info BLOB,hide_attachment INTEGER,emoji_image_content_identifier TEXT,emoji_image_short_description TEXT); CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER);")
        try db.execute("INSERT INTO chat VALUES('synthetic-chat',1,X'" + fixture("rich-chat.plist") + "'); INSERT INTO message(guid,balloon_bundle_id,payload_data,attributedBody,is_audio_message,date_played,is_expirable,schedule_type,schedule_state,date,subject,was_delivered_quietly,did_notify_recipient,is_spam,has_unseen_mention,is_stewie,is_sos) VALUES('synthetic-rich','com.apple.messages.URLBalloonProvider',X'" + fixture("rich-link.plist") + "',X'" + fixture("rich-formatting.typedstream") + "',1,800000000,1,2,3,800000000,'Synthetic subject',1,1,1,1,1,1); INSERT INTO chat_recoverable_message_join VALUES(1,1,800000001);")
        try db.execute("INSERT INTO message(guid,attributedBody,associated_message_guid) VALUES('synthetic-genmoji-message',X'" + fixture("rich-genmoji.typedstream") + "','p:2/synthetic-rich'); INSERT INTO attachment VALUES('synthetic-genmoji','',1,X'" + fixture("rich-sticker.plist") + "',1,'synthetic-glyph','Synthetic glyph'); INSERT INTO message_attachment_join VALUES(2,1);")
        let source = Messages(path: path)
        let readiness = await source.open()
        try MessagesCaptureChecks.require(readiness.state == "ready", "rich source readiness")
        let sink = CaptureSink()
        let task = Task { try await source.capture(.null, into: sink) }
        defer { task.cancel() }
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait { await sink.completed() }, "rich capture complete")
        let m = await sink.record("synthetic-rich")
        try MessagesCaptureChecks.require(m["chat_id"]["$ref"]["native_key"].string == "synthetic-chat", "recoverable membership supplies message chat")
        try MessagesCaptureChecks.require(m["balloon"]["kind"].string == "link" && m["balloon"]["link"]["title"].string == "Synthetic link", "captured link archive")
        try MessagesCaptureChecks.require(m["audio"]["is_audio_message"].bool && m["audio"]["expirable"].bool && !m["audio"]["played_at"].string.isEmpty && !m["schedule"]["scheduled_for"].string.isEmpty, "captured audio schedule")
        try MessagesCaptureChecks.require(m["subject"].string == "Synthetic subject" && m["delivery"]["quietly"].bool && m["delivery"]["notified_anyway"].bool && m["is_spam"].bool && m["has_unseen_mention"].bool && m["via_satellite"].bool && m["is_sos"].bool, "captured message flags")
        try MessagesCaptureChecks.require(!m["formatting"].array.isEmpty && !m["deleted_at"].string.isEmpty, "captured formatting deletion")
        let a = await sink.record("synthetic-genmoji")
        try MessagesCaptureChecks.require(a["is_sticker"].bool && a["hidden"].bool && a["genmoji"].bool && a["sticker"]["part_index"].number == 2, "captured sticker hidden genmoji")
        let chat = await sink.record("synthetic-chat")
        try MessagesCaptureChecks.require(chat["filter"].string == "unknown" && chat["read_receipts"].string == "on" && !chat["group_photo_attachment_id"]["$ref"]["native_key"].string.isEmpty, "captured chat properties")
        try db.execute("UPDATE message SET subject='Synthetic changed',was_delivered_quietly=0,has_unseen_mention=0 WHERE guid='synthetic-rich';")
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait(2) { let current = await sink.record("synthetic-rich"); return current["subject"].string == "Synthetic changed" && !current["delivery"]["quietly"].bool && !current["has_unseen_mention"].bool }, "live rich mutation")
        task.cancel()
        _ = try? await task.value
        print("all rich Messages capture checks passed")
    }
}
