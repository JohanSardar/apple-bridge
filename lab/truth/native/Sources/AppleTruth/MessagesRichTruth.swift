import Foundation
import SQLite3

enum MessagesRichTruth {
    static func observe(_ request: [String: Any]) throws -> [String: Any] {
        let path = request["snapshot_path"] as? String ?? NSHomeDirectory() + "/Library/Messages/chat.db"
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if db != nil { sqlite3_close(db) }; throw TruthFailure.permissionMissing }
        defer { sqlite3_close(db) }
        func count(_ table: String, _ predicate: String) throws -> Int {
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT COUNT(*) FROM " + table + " WHERE " + predicate, -1, &stmt, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
            defer { sqlite3_finalize(stmt) }
            guard sqlite3_step(stmt) == SQLITE_ROW else { throw TruthFailure.unavailable }
            return Int(sqlite3_column_int64(stmt, 0))
        }
        let message = "message m"
        let ordinary = "NOT(COALESCE(associated_message_type,0) BETWEEN 2000 AND 2007 OR COALESCE(associated_message_type,0) BETWEEN 3000 AND 3007)"
        var counts: [String: Int] = [:]
        let kinds = ["link", "app", "digital_touch", "handwriting", "poll", "check_in", "apple_cash", "find_my", "chatbot", "unknown"]
        for kind in kinds { counts["balloon." + kind] = 0 }
        for key in ["balloon.payload_decoded", "balloon.link_card", "balloon.app_caption", "balloon.image_reference"] { counts[key] = 0 }
        for key in ["title", "summary", "site_name", "video_url", "icon_attachment_id"] { counts["balloon.link." + key] = 0 }
        for key in ["subcaption", "trailing_caption", "url"] { counts["balloon.app." + key] = 0 }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT balloon_bundle_id,payload_data,ROWID FROM message WHERE COALESCE(balloon_bundle_id,'')!='' AND " + ordinary, -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        while sqlite3_step(statement) == SQLITE_ROW {
            let bundle = String(cString: sqlite3_column_text(statement, 0)).lowercased()
            let kind: String
            if bundle.contains("urlballoonprovider") { kind = "link" }
            else if bundle.contains("digitaltouch") { kind = "digital_touch" }
            else if bundle.contains("handwriting") { kind = "handwriting" }
            else if bundle.contains("peerpayment") || bundle.contains("applecash") { kind = "apple_cash" }
            else if bundle.contains("findmy") { kind = "find_my" }
            else if bundle.contains("safetymonitor") || bundle.contains("checkin") { kind = "check_in" }
            else if bundle.contains("poll") { kind = "poll" }
            else if bundle.contains("chatbot") { kind = "chatbot" }
            else if bundle.contains("msmessageextensionballoonplugin") { kind = "app" }
            else { kind = "unknown" }
            counts["balloon." + kind, default: 0] += 1
            if let pointer = sqlite3_column_blob(statement, 1), let archive = TruthArchive(Data(bytes: pointer, count: Int(sqlite3_column_bytes(statement, 1)))) {
                counts["balloon.payload_decoded", default: 0] += 1
                let metadata = archive.field(archive.root, "richLinkMetadata") ?? archive.root
                let specialization = archive.field(metadata,"specialization")
                if kind == "link", archive.field(specialization,"address") != nil || archive.field(specialization,"addressComponents") != nil || archive.field(specialization,"isPointOfInterest") != nil {
                    counts["balloon.link",default:0] -= 1
                    counts["balloon.find_my",default:0] += 1
                }
                let link = kind == "link" && (archive.text(metadata, "URL") ?? archive.text(metadata, "originalURL")) != nil
                if link {
                    counts["balloon.link_card", default: 0] += 1
                    for (source, field) in [("title", "title"), ("summary", "summary"), ("siteName", "site_name")] where archive.text(metadata, source) != nil { counts["balloon.link." + field, default: 0] += 1 }
                    let video = archive.field(metadata, "video") ?? archive.field(metadata, "videoMetadata")
                    if (archive.text(video, "URL") ?? archive.text(metadata, "youTubeURL")) != nil { counts["balloon.link.video_url", default: 0] += 1 }
                    if let index = archive.field(archive.field(metadata, "icon"), "richLinkImageAttachmentSubstituteIndex") as? Int {
                        let icons = try count("attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID", "j.message_id=" + String(sqlite3_column_int64(statement, 2)) + " AND (a.mime_type LIKE 'image/%' OR a.uti LIKE 'dyn.age81%')")
                        if index >= 0 && index < icons { counts["balloon.link.icon_attachment_id", default: 0] += 1 }
                    }
                }
                if kind != "link" {
                    let layout = archive.field(archive.root, "layout") ?? archive.root
                    if (archive.text(layout, "caption") ?? archive.text(layout, "ldtext")) != nil { counts["balloon.app_caption", default: 0] += 1 }
                    for (source, field) in [("subcaption", "subcaption"), ("secondary-subcaption", "trailing_caption"), ("URL", "url")] where archive.text(layout, source) != nil { counts["balloon.app." + field, default: 0] += 1 }
                }
                if kind == "link" {
                    let images = try count("attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID", "j.message_id=" + String(sqlite3_column_int64(statement, 2)) + " AND (a.mime_type LIKE 'image/%' OR a.uti LIKE 'dyn.age81%')")
                    if link, let index = archive.field(archive.field(metadata, "image"), "richLinkImageAttachmentSubstituteIndex") as? Int, index >= 0, index < images { counts["balloon.image_reference", default: 0] += 1 }
                }
            }
            if kind != "link" {
                let images = try count("attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID", "j.message_id=" + String(sqlite3_column_int64(statement, 2)) + " AND (a.mime_type LIKE 'image/%' OR a.uti LIKE 'dyn.age81%')")
                if images > 0 { counts["balloon.image_reference", default: 0] += 1 }
            }
        }
        sqlite3_finalize(statement)
        let predicates = ["audio": "is_audio_message!=0", "audio.played": "is_audio_message!=0 AND date_played>0", "audio.expirable": "is_audio_message!=0 AND is_expirable!=0", "schedule": "schedule_type!=0", "subject": "COALESCE(subject,'')!=''", "delivery.quietly": "was_delivered_quietly!=0", "delivery.notified_anyway": "did_notify_recipient!=0", "is_spam": "is_spam!=0", "has_unseen_mention": "has_unseen_mention!=0", "via_satellite": "(is_stewie!=0 OR sent_or_received_off_grid!=0)", "is_sos": "is_sos!=0", "formatting": "instr(attributedBody,'__kIMText')>0", "genmoji": "instr(attributedBody,'__kIMEmojiImageAttributeName')>0"]
        for (key, predicate) in predicates { counts[key] = try count(message, "(" + predicate + ") AND " + ordinary) }
        counts["attachment.is_sticker"] = try count("attachment", "is_sticker!=0")
        counts["attachment.hidden"] = try count("attachment", "hide_attachment!=0")
        counts["attachment.genmoji"] = try count("attachment", "COALESCE(emoji_image_content_identifier,'')!=''")
        counts["chat.unknown"] = try count("chat", "is_filtered!=0")
        counts["chat.is_spam"] = try count("chat", "is_filtered=2")
        counts["message.deleted_at"] = try count("message", "EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=message.ROWID AND r.delete_date>0) AND NOT EXISTS(SELECT j.chat_id FROM chat_message_join j WHERE j.message_id=message.ROWID EXCEPT SELECT r.chat_id FROM chat_recoverable_message_join r WHERE r.message_id=message.ROWID) AND " + ordinary)
        counts["chat.deleted_at"] = try count("chat c", "EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.delete_date>0) AND NOT EXISTS(SELECT 1 FROM chat_message_join j WHERE j.chat_id=c.ROWID AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=j.message_id))")
        counts["chat.group_photo"] = 0
        counts["chat.read_receipts"] = 0
        guard sqlite3_prepare_v2(db, "SELECT properties FROM chat WHERE properties IS NOT NULL", -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        while sqlite3_step(statement) == SQLITE_ROW {
            guard let bytes = sqlite3_column_blob(statement, 0), let p = (try? PropertyListSerialization.propertyList(from: Data(bytes: bytes, count: Int(sqlite3_column_bytes(statement, 0))), format: nil)) as? [String: Any] else { continue }
            if let photo = p["groupPhotoGuid"] as? String, !photo.isEmpty { counts["chat.group_photo", default: 0] += 1 }
            if p["EnableReadReceiptForChat"] is NSNumber { counts["chat.read_receipts", default: 0] += 1 }
        }
        sqlite3_finalize(statement)
        counts["attachment.sticker"] = 0
        let sql = "SELECT a.sticker_user_info FROM attachment a WHERE a.sticker_user_info IS NOT NULL AND EXISTS(SELECT 1 FROM message_attachment_join j JOIN message m ON m.ROWID=j.message_id WHERE j.attachment_id=a.ROWID AND COALESCE(m.associated_message_guid,'')!='')"
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        while sqlite3_step(statement) == SQLITE_ROW {
            guard let bytes = sqlite3_column_blob(statement, 0), let p = (try? PropertyListSerialization.propertyList(from: Data(bytes: bytes, count: Int(sqlite3_column_bytes(statement, 0))), format: nil)) as? [String: Any] else { continue }
            if ["sxs", "sys", "ssa", "sro"].allSatisfy({ Double(String(describing: p[$0] ?? ""))?.isFinite == true }) { counts["attachment.sticker", default: 0] += 1 }
        }
        sqlite3_finalize(statement)
        return ["status": "verified", "counts": counts]
    }
}

private struct TruthArchive {
    let objects: [Any]
    let root: Any
    init?(_ data: Data) {
        guard let plist = try? PropertyListSerialization.propertyList(from: data, format: nil),
              let xml = try? PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0),
              let text = String(data: xml, encoding: .utf8),
              let plain = text.replacingOccurrences(of: "<key>CF$UID</key>", with: "<key>truth_uid</key>").data(using: .utf8),
              let normalized = try? PropertyListSerialization.propertyList(from: plain, format: nil) as? [String: Any] else { return nil }
        objects = normalized["$objects"] as? [Any] ?? []
        root = (normalized["$top"] as? [String: Any])?["root"] ?? normalized
    }
    func resolve(_ value: Any?, depth: Int = 0) -> Any? {
        guard depth < 64, let value else { return nil }
        if let reference = value as? [String: Any], let slot = reference["truth_uid"] as? Int {
            guard slot > 0, objects.indices.contains(slot) else { return nil }
            return resolve(objects[slot], depth: depth + 1)
        }
        return value
    }
    func field(_ object: Any?, _ key: String) -> Any? {
        guard let dictionary = resolve(object) as? [String: Any] else { return nil }
        if let keys = resolve(dictionary["NS.keys"]) as? [Any], let values = resolve(dictionary["NS.objects"]) as? [Any], keys.count == values.count {
            for index in keys.indices where resolve(keys[index]) as? String == key { return resolve(values[index]) }
        }
        return resolve(dictionary[key])
    }
    func text(_ object: Any?, _ key: String) -> String? {
        if let text = field(object, key) as? String { return text }
        return field(field(object, key), "NS.relative") as? String
    }
}
