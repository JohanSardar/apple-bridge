import Foundation
import SQLite3
import Decoders

struct SQLiteRow {
    var values: [String: String] = [:]
    var blobs: [String: Data] = [:]
    subscript(_ column: String) -> String { values[column] ?? "" }
    func integer(_ column: String) -> Int64 { Int64(self[column]) ?? 0 }
    func flag(_ column: String) -> Bool { integer(column) != 0 }
}

final class MessagesReader {
    let path: String
    private let dateFormat: ISO8601DateFormatter = { let value = ISO8601DateFormatter(); value.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return value }()
    private var connection: OpaquePointer?
    private var identity = ""
    private var walIdentity: UInt64?
    private var schemaVersion: Int64 = 0
    private(set) var generation = ""
    private(set) var columns: [String: Set<String>] = [:]
    init(path: String) throws {
        self.path = path
        guard sqlite3_open_v2(path, &connection, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            if let connection { sqlite3_close(connection) }
            connection = nil
            throw ProbeFailure.unavailable
        }
        sqlite3_busy_timeout(connection, 250)
        var schema: [String: Set<String>] = [:]
        for table in ["message", "chat", "handle", "chat_handle_join", "chat_message_join", "message_attachment_join", "attachment", "chat_recoverable_message_join", "recoverable_message_part"] {
            schema[table] = Set(try query("PRAGMA table_info(\(table))").map { $0["name"] })
        }
        guard schema["message"]?.contains("guid") == true else { throw ProbeFailure.unavailable }
        columns = schema
        let attributes = try FileManager.default.attributesOfItem(atPath: path)
        identity = "\(attributes[.systemNumber] ?? 0):\(attributes[.systemFileNumber] ?? 0)"
        walIdentity = ((try? FileManager.default.attributesOfItem(atPath: path + "-wal")[.systemFileNumber]) as? NSNumber)?.uint64Value
        schemaVersion = try query("PRAGMA schema_version").first?.integer("schema_version") ?? 0
        let signature = schema.keys.sorted().map { $0 + ":" + schema[$0]!.sorted().joined(separator: ",") }.joined(separator: "\n")
        generation = BodyDigest.hash(Data((identity + signature).utf8))
    }
    deinit { sqlite3_close(connection) }
    func query(_ sql: String, _ bindings: [String] = []) throws -> [SQLiteRow] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, sql, -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        for (offset, value) in bindings.enumerated() {
            let status = value.withCString { sqlite3_bind_text(statement, Int32(offset + 1), $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard status == SQLITE_OK else { throw ProbeFailure.unavailable }
        }
        var rows: [SQLiteRow] = []
        while true {
            let result = sqlite3_step(statement)
            if result == SQLITE_DONE { return rows }
            guard result == SQLITE_ROW else { throw ProbeFailure.unavailable }
            var row = SQLiteRow()
            for index in 0..<sqlite3_column_count(statement) {
                let name = String(cString: sqlite3_column_name(statement, index))
                switch sqlite3_column_type(statement, index) {
                case SQLITE_NULL: break
                case SQLITE_BLOB:
                    let count = Int(sqlite3_column_bytes(statement, index))
                    if count == 0 { row.blobs[name] = Data() }
                    else if let bytes = sqlite3_column_blob(statement, index) { row.blobs[name] = Data(bytes: bytes, count: count) }
                default:
                    if let text = sqlite3_column_text(statement, index) {
                        let data = Data(bytes: text, count: Int(sqlite3_column_bytes(statement, index)))
                        if let value = String(data: data, encoding: .utf8) { row.values[name] = value }
                        else { row.blobs[name] = data }
                    }
                }
            }
            rows.append(row)
        }
    }
    func isCurrent() throws -> Bool {
        let attributes = try FileManager.default.attributesOfItem(atPath: path)
        let current = "\(attributes[.systemNumber] ?? 0):\(attributes[.systemFileNumber] ?? 0)"
        guard current == identity else { return false }
        let currentWAL = ((try? FileManager.default.attributesOfItem(atPath: path + "-wal")[.systemFileNumber]) as? NSNumber)?.uint64Value
        guard currentWAL == walIdentity else { return false }
        return (try query("PRAGMA schema_version").first?.integer("schema_version") ?? 0) == schemaVersion
    }
    func dataVersion() throws -> Int64 { try query("PRAGMA data_version").first?.integer("data_version") ?? 0 }
    func maximum() throws -> Int64 { try query("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM message").first?.integer("maximum") ?? 0 }
    private func has(_ table: String, _ fields: String...) -> Bool {
        fields.allSatisfy { columns[table]?.contains($0) == true }
    }
    private var recoverySupported: Bool {
        has("chat_recoverable_message_join", "chat_id", "message_id", "delete_date") && has("chat_message_join", "chat_id", "message_id")
    }
    private var messageDeletionDate: String {
        recoverySupported ? "(SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND NOT EXISTS(SELECT 1 FROM chat_message_join j WHERE j.message_id=m.ROWID AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join visible WHERE visible.chat_id=j.chat_id AND visible.message_id=m.ROWID)))" : "NULL"
    }
    private var chatDeletionDate: String {
        recoverySupported ? "(SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND NOT EXISTS(SELECT 1 FROM chat_message_join j WHERE j.chat_id=c.ROWID AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join visible WHERE visible.chat_id=c.ROWID AND visible.message_id=j.message_id)))" : "NULL"
    }
    func recoverableChat(_ guid: String) throws -> Bool {
        let row = try query("SELECT " + chatDeletionDate + " AS deleted_date FROM chat c WHERE c.guid=?", [guid]).first
        return (Double(row?["deleted_date"] ?? "") ?? 0) > 0
    }
    private var messageProjection: String {
        let activeChat = has("chat", "guid") && has("chat_message_join", "chat_id", "message_id") ? "(SELECT c.guid FROM chat_message_join j JOIN chat c ON c.ROWID=j.chat_id WHERE j.message_id=m.ROWID ORDER BY c.guid LIMIT 1)" : "NULL"
        let recoverableChat = has("chat", "guid") && has("chat_recoverable_message_join", "chat_id", "message_id") ? "(SELECT c.guid FROM chat_recoverable_message_join j JOIN chat c ON c.ROWID=j.chat_id WHERE j.message_id=m.ROWID ORDER BY c.guid LIMIT 1)" : "NULL"
        let chat = "COALESCE(\(activeChat),\(recoverableChat))"
        let sender = has("message", "handle_id") && has("handle", "service", "id") ? "(SELECT h.service || ':' || h.id FROM handle h WHERE h.ROWID=m.handle_id)" : "NULL"
        let member = has("message", "other_handle") && has("handle", "service", "id") ? "(SELECT h.service || ':' || h.id FROM handle h WHERE h.ROWID=m.other_handle)" : "NULL"
        let attachmentFields = ["guid", "filename", "mime_type", "uti", "emoji_image_content_identifier", "emoji_image_short_description"].filter { columns["attachment"]?.contains($0) == true }
        let attachmentValues = attachmentFields.map { "quote(a.\($0))" }.joined(separator: "||':'||")
        let attachments = has("message_attachment_join", "message_id", "attachment_id") && has("attachment", "guid") ? "(SELECT group_concat(signature,'|') FROM (SELECT \(attachmentValues) AS signature FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID WHERE j.message_id=m.ROWID ORDER BY a.ROWID))" : "NULL"
        return "m.ROWID AS row_position,m.*,\(chat) AS capture_chat_guid,\(sender) AS capture_sender_key,\(messageDeletionDate) AS capture_deleted_date,\(attachments) AS capture_attachment_signature,\(member) AS capture_member_key"
    }
    func messageRows(after: Int64, through: Int64, limit: Int = 128) throws -> [SQLiteRow] {
        try projectedRows("SELECT ROWID AS row_position FROM message WHERE ROWID>? AND ROWID<=? ORDER BY ROWID LIMIT ?", [String(after), String(through), String(limit)])
    }
    func recoverableGUIDs() throws -> Set<String> {
        guard has("chat_recoverable_message_join", "message_id") else { return [] }
        return Set(try query("SELECT DISTINCT m.guid FROM message m JOIN chat_recoverable_message_join r ON r.message_id=m.ROWID").map { $0["guid"] })
    }
    private func projectedRows(_ selection: String, _ bindings: [String], descending: Bool = false) throws -> [SQLiteRow] {
        let joins = ["chat_message_join", "chat_recoverable_message_join", "message_attachment_join"].filter { columns[$0]?.contains("message_id") == true }
        let tables = ["capture_batch AS MATERIALIZED (\(selection))"] + joins.map {
            "\($0) AS MATERIALIZED (SELECT * FROM main.\($0) WHERE message_id IN (SELECT row_position FROM capture_batch))"
        }
        let order = descending ? "DESC" : "ASC"
        return try query("WITH " + tables.joined(separator: ",") + " SELECT \(messageProjection) FROM message m WHERE m.ROWID IN (SELECT row_position FROM capture_batch) ORDER BY m.ROWID \(order)", bindings)
    }
    func recent() throws -> [SQLiteRow] { try projectedRows("SELECT ROWID AS row_position FROM message ORDER BY ROWID DESC LIMIT 2000", [], descending: true) }
    func messages(_ guids: [String]) throws -> [SQLiteRow] {
        guard !guids.isEmpty else { return [] }
        let placeholders = Array(repeating: "?", count: guids.count).joined(separator: ",")
        return try projectedRows("SELECT ROWID AS row_position FROM message WHERE guid IN (\(placeholders))", guids)
    }
    func existingMessages(_ guids: [String]) throws -> Set<String> {
        guard !guids.isEmpty else { return [] }
        let placeholders = Array(repeating: "?", count: guids.count).joined(separator: ",")
        return Set(try query("SELECT guid FROM message WHERE guid IN (\(placeholders))", guids).map { $0["guid"] })
    }
    func message(_ guid: String) throws -> SQLiteRow? { try query("SELECT \(messageProjection) FROM message m WHERE m.guid=?", [guid]).first }
    func reference(_ collection: String, _ guid: String) -> [String: Any] { ["$ref": ["collection": "messages." + collection, "native_key": guid]] }
    func date(_ row: SQLiteRow, _ column: String) -> Date? {
        guard let raw = Double(row[column]), raw > 0 else { return nil }
        return Date(timeIntervalSinceReferenceDate: raw > 1e12 ? raw / 1e9 : raw)
    }
    func timestamp(_ row: SQLiteRow, _ column: String) -> Any? { date(row, column).map { dateFormat.string(from: $0) } }
    private func targetGUID(_ value: String) -> String {
        if value.hasPrefix("p:"), let slash = value.firstIndex(of: "/") { return String(value[value.index(after: slash)...]) }
        return value.hasPrefix("bp:") ? String(value.dropFirst(3)) : value
    }
    func messageObservations(_ row: SQLiteRow) throws -> [(String, String, [String: Any])] {
        let guid = row["guid"]
        guard !guid.isEmpty else { return [] }
        let chat = row["capture_chat_guid"]
        let senderKey = row.values["capture_sender_key"]
        if let reaction = MessageReaction.decode(type: Int(row.integer("associated_message_type")), target: row["associated_message_guid"], emoji: row.values["associated_message_emoji"]) {
            var data: [String: Any] = ["message_id": reference("message", reaction.messageGUID), "part_index": reaction.partIndex, "kind": reaction.kind, "removed": reaction.removed, "is_from_me": row.flag("is_from_me")]
            data["sent_at"] = timestamp(row, "date")
            data["emoji"] = reaction.emoji
            data["sender_handle_id"] = senderKey.map { reference("handle", $0) }
            return [("messages.reaction", guid, data)]
        }
        var data: [String: Any] = ["is_from_me": row.flag("is_from_me"), "kind": row.integer("item_type") == 0 ? "message" : "group_event", "error_code": row.integer("error"), "edit_history": [], "retracted_parts": [], "formatting": []]
        data["chat_id"] = chat.isEmpty ? nil : reference("chat", chat)
        data["deleted_at"] = timestamp(row, "capture_deleted_date")
        data["sender_handle_id"] = senderKey.map { reference("handle", $0) }
        data["service"] = row["service"].isEmpty ? nil : row["service"]
        for (native, field) in [("date", "sent_at"), ("date_delivered", "delivered_at"), ("date_read", "read_at"), ("date_edited", "edited_at"), ("date_retracted", "retracted_at")] { data[field] = timestamp(row, native) }
        for (native, field) in [("reply_to_guid", "reply_to_id"), ("thread_originator_guid", "thread_originator_id")] {
            if !row[native].isEmpty { data[field] = reference("message", targetGUID(row[native])) }
        }
        var parts: [[String: Any]] = []
        if let bytes = row.blobs["attributedBody"] {
            let body = TypedStream.decode(bytes)
            data["decode_status"] = body.decodeStatus
            data["raw_body_digest"] = body.rawBodyDigest
            data["text"] = body.text
            data["formatting"] = body.formatting.map { range -> [String: Any] in
                var value: [String: Any] = ["utf16_location": range.utf16Location, "utf16_length": range.utf16Length, "styles": range.styles]
                value["effect"] = range.effect
                return value
            }
            for part in body.parts {
                var value: [String: Any] = ["index": part.index, "kind": part.kind, "utf16_location": part.utf16Location, "utf16_length": part.utf16Length]
                value["text"] = part.text
                value["mention"] = part.mention
                value["attachment_id"] = part.attachmentGUID.map { reference("attachment", $0) }
                parts.append(value)
            }
        } else if let text = row.values["text"] {
            data["text"] = text
            data["decode_status"] = "decoded"
            parts = [["index": 0, "kind": "text", "text": text]]
        } else {
            data["decode_status"] = row.integer("date_retracted") > 0 || row.integer("item_type") != 0 ? "decoded" : "undecoded"
            data["raw_body_digest"] = BodyDigest.hash(row.blobs["text"] ?? Data())
        }
        data["parts"] = parts
        data["subject"] = row.values["subject"]
        data["delivery"] = ["quietly": row.flag("was_delivered_quietly"), "notified_anyway": row.flag("did_notify_recipient")]
        for (native, field) in [("is_spam", "is_spam"), ("has_unseen_mention", "has_unseen_mention"), ("is_sos", "is_sos")] { data[field] = row.flag(native) }
        data["via_satellite"] = row.flag("is_stewie") || row.flag("sent_or_received_off_grid")
        var audio: [String: Any] = ["is_audio_message": row.flag("is_audio_message"), "expirable": row.flag("is_expirable"), "kept": NSNull()]
        audio["played_at"] = timestamp(row, "date_played")
        data["audio"] = audio
        if row.integer("schedule_type") != 0 {
            var schedule: [String: Any] = ["state": "native_" + row["schedule_state"]]
            schedule["scheduled_for"] = timestamp(row, "date")
            data["schedule"] = schedule
        }
        if data["deleted_at"] != nil { data["recoverable_until"] = NSNull() }
        if !row["balloon_bundle_id"].isEmpty || parts.contains(where: { $0["kind"] as? String == "genmoji" }) {
            let attachments = try has("message_attachment_join", "attachment_id", "message_id") ? query("SELECT a.* FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID WHERE j.message_id=? ORDER BY a.ROWID", [row["row_position"]]) : []
            let images = attachments.filter { imageMIME($0).hasPrefix("image/") || $0["uti"].hasPrefix("dyn.age81") }.map { $0["guid"] }
            if var balloon = MessageRich.balloon(bundle: row["balloon_bundle_id"], payload: row.blobs["payload_data"], attachmentGUIDs: images) {
                for key in ["link", "app"] {
                    if var card = balloon[key] as? [String: Any] {
                        for field in ["image_attachment_id", "icon_attachment_id"] { if let guid = card[field] as? String { card[field] = reference("attachment", guid) } }
                        balloon[key] = card
                    }
                }
                data["balloon"] = balloon
            }
            for i in parts.indices where parts[i]["kind"] as? String == "genmoji" {
                let target = ((parts[i]["attachment_id"] as? [String: Any])?["$ref"] as? [String: String])?["native_key"]
                let glyphs = attachments.filter { !$0["emoji_image_content_identifier"].isEmpty }
                if let attachment = glyphs.first(where: { $0["guid"] == target }) ?? (glyphs.count == 1 ? glyphs.first : nil) {
                    parts[i]["attachment_id"] = reference("attachment", attachment["guid"])
                    parts[i]["description"] = attachment["emoji_image_short_description"]
                }
            }
            data["parts"] = parts
        }
        if !row["expressive_send_style_id"].isEmpty { data["effect"] = row["expressive_send_style_id"] }
        if let bytes = row.blobs["message_summary_info"] {
            let summary = MessageSummary.decode(bytes)
            data["edit_history"] = summary.editHistory.map { edit in
                var value: [String: Any] = ["part_index": edit.partIndex, "decode_status": edit.body.decodeStatus]
                value["text"] = edit.body.text
                value["timestamp"] = edit.timestamp.map { dateFormat.string(from: Date(timeIntervalSinceReferenceDate: $0)) }
                value["raw_body_digest"] = edit.body.rawBodyDigest
                return value
            }
            data["retracted_parts"] = summary.retractedParts
            if summary.decodeStatus != "decoded" {
                data["summary_decode_status"] = summary.decodeStatus
                data["summary_raw_digest"] = summary.rawBodyDigest
                if data["decode_status"] as? String == "decoded" { data["decode_status"] = "partial" }
            }
        }
        if let window = MessageActionWindow(service: row["service"], isFromMe: row.flag("is_from_me"), isMessage: row.integer("item_type") == 0, sentAt: date(row, "date"), edits: (data["edit_history"] as? [Any])?.count ?? 0) {
            data["edit_until"] = window.editUntil.map { dateFormat.string(from: $0) }
            data["unsend_until"] = dateFormat.string(from: window.unsendUntil)
        }
        if row.integer("item_type") != 0 {
            var event: [String: Any] = ["item_type": row.integer("item_type"), "action_type": row.integer("group_action_type"), "title": row["group_title"], "kind": MessageRich.groupEventKind(item: Int(row.integer("item_type")), action: Int(row.integer("group_action_type")), hasMember: row.integer("other_handle") > 0, senderIsMember: row.integer("handle_id") == row.integer("other_handle"), title: row["group_title"])]
            if row.integer("item_type") == 1, !row["capture_member_key"].isEmpty { event["target_handle_id"] = reference("handle", row["capture_member_key"]) }
            event["actor_handle_id"] = senderKey.map { reference("handle", $0) }
            data["group_event"] = event
        }
        return [("messages.message", guid, data)]
    }
    func smallObservations() throws -> [(String, String, [String: Any])] {
        let muteProfile = MessagesParity.muteProfile()
        let pins = try? MessagesPins.read()
        var result: [(String, String, [String: Any])] = []
        for row in try has("handle", "id", "service") ? query("SELECT ROWID AS row_position,* FROM handle") : [] {
            result.append(("messages.handle", row["service"] + ":" + row["id"], ["address": row["id"], "service": row["service"]]))
        }
        for row in try has("chat", "guid") ? query("SELECT c.ROWID AS row_position,c.*,\(chatDeletionDate) AS capture_deleted_date FROM chat c") : [] {
            let participants = try has("handle", "id", "service") && has("chat_handle_join", "chat_id", "handle_id") ? query("SELECT h.id,h.service FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id WHERE j.chat_id=? ORDER BY h.service,h.id", [row["row_position"]]).map { reference("handle", $0["service"] + ":" + $0["id"]) } : []
            var data: [String: Any] = ["service": row["service_name"], "style": row["guid"].contains(";+;") ? "group" : "direct", "display_name": row["display_name"], "participants": participants, "is_archived": row.flag("is_archived"), "capabilities": []]
            data["last_read_at"] = timestamp(row, "last_read_message_timestamp")
            data["deleted_at"] = timestamp(row, "capture_deleted_date")
            if data["deleted_at"] != nil { data["recoverable_until"] = NSNull() }
            data["filter"] = row.flag("is_filtered") ? "unknown" : "known"
            data["is_spam"] = row.integer("is_filtered") == 2
            for (key, value) in MessageRich.chatProperties(row.blobs["properties"]) { data[key] = value }
            for (key, value) in MessagesParity.capturedFields(row.blobs["properties"], service: row["service_name"], profile: muteProfile) { data[key] = value }
            if let pins, let index = MessagesPins.index(row["chat_identifier"], in: pins) { data["pinned_index"] = index }
            if let photo = data["group_photo_attachment_id"] as? String { data["group_photo_attachment_id"] = reference("attachment", photo) }
            result.append(("messages.chat", row["guid"], data))
        }
        return result + (try attachmentObservations())
    }
    func attachmentObservations(guid: String? = nil) throws -> [(String, String, [String: Any])] {
        guard has("attachment", "guid") else { return [] }
        var result: [(String, String, [String: Any])] = []
        let message = has("message_attachment_join", "message_id", "attachment_id") ? "(SELECT m.guid FROM message_attachment_join j JOIN message m ON m.ROWID=j.message_id WHERE j.attachment_id=a.ROWID ORDER BY m.ROWID LIMIT 1)" : "NULL"
        for row in try query("SELECT a.ROWID AS row_position,a.*,\(message) AS message_guid FROM attachment a" + (guid == nil ? " ORDER BY a.ROWID DESC" : " WHERE a.guid=?"), guid.map { [$0] } ?? []) {
            let location = attachmentURL(row["filename"])
            let exists = location.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
            let resource = try? location?.resourceValues(forKeys: [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey])
            let offloaded = (resource?.isUbiquitousItem == true && resource?.ubiquitousItemDownloadingStatus == .notDownloaded) || location.map(BlobDigest.isDataless) == true
            let state = offloaded ? "offloaded" : exists ? "present" : row.integer("transfer_state") == 0 ? "pending" : row["filename"].isEmpty ? "missing" : "offloaded"
            var blob: [String: Any] = ["state": state]
            if exists { blob["handle"] = row["guid"] }
            var data: [String: Any] = ["filename": URL(fileURLWithPath: row["filename"]).lastPathComponent, "mime_type": imageMIME(row), "uti": row["uti"], "size": row.integer("total_bytes"), "blob": blob]
            if !row["message_guid"].isEmpty { data["message_id"] = reference("message", row["message_guid"]) }
            data["is_sticker"] = row.flag("is_sticker")
            data["hidden"] = row.flag("hide_attachment")
            data["genmoji"] = !row["emoji_image_content_identifier"].isEmpty
            if !row["emoji_image_short_description"].isEmpty { data["description"] = row["emoji_image_short_description"] }
            if let bytes = row.blobs["sticker_user_info"], has("message", "associated_message_guid"),
               let parent = try query("SELECT associated_message_guid FROM message WHERE guid=?", [row["message_guid"]]).first,
               var sticker = MessageRich.sticker(bytes, target: parent["associated_message_guid"]) {
                if let target = sticker["target_message_id"] as? String { sticker["target_message_id"] = reference("message", target) }
                data["sticker"] = sticker
            }
            result.append(("messages.attachment", row["guid"], data))
        }
        return result
    }
    private func imageMIME(_ row: SQLiteRow) -> String {
        if !row["uti"].hasPrefix("dyn.age81") { return row["mime_type"] }
        guard row["uti"].hasPrefix("dyn.age81"), let url = attachmentURL(row["filename"]), !BlobDigest.isDataless(url), let file = try? FileHandle(forReadingFrom: url) else { return row["mime_type"] }
        defer { try? file.close() }
        guard let bytes = try? file.read(upToCount: 12) else { return row["mime_type"] }
        if bytes.starts(with: [137,80,78,71,13,10,26,10]) { return "image/png" }
        if bytes.starts(with: [255,216,255]) { return "image/jpeg" }
        if bytes.starts(with: Data("GIF8".utf8)) { return "image/gif" }
        return row["mime_type"]
    }
    func attachmentURL(_ path: String) -> URL? {
        guard !path.isEmpty else { return nil }
        let expanded = (path as NSString).expandingTildeInPath
        let url = URL(fileURLWithPath: expanded).standardizedFileURL.resolvingSymlinksInPath()
        let root = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/Attachments").resolvingSymlinksInPath().path + "/"
        return url.path.hasPrefix(root) ? url : nil
    }
}
