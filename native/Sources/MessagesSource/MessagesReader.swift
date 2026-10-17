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
        for table in ["message", "chat", "handle", "chat_handle_join", "chat_message_join", "message_attachment_join", "attachment"] {
            schema[table] = Set(try query("PRAGMA table_info(\(table))").map { $0["name"] })
        }
        guard schema["message"]?.contains("guid") == true, schema["chat"]?.contains("guid") == true else { throw ProbeFailure.unavailable }
        columns = schema
        let attributes = try FileManager.default.attributesOfItem(atPath: path)
        identity = "\(attributes[.systemNumber] ?? 0):\(attributes[.systemFileNumber] ?? 0)"
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
        return (try query("PRAGMA schema_version").first?.integer("schema_version") ?? 0) == schemaVersion
    }
    func dataVersion() throws -> Int64 { try query("PRAGMA data_version").first?.integer("data_version") ?? 0 }
    func maximum() throws -> Int64 { try query("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM message").first?.integer("maximum") ?? 0 }
    private var messageProjection: String {
        "m.ROWID AS row_position,m.*,(SELECT c.guid FROM chat_message_join j JOIN chat c ON c.ROWID=j.chat_id WHERE j.message_id=m.ROWID ORDER BY c.guid LIMIT 1) AS capture_chat_guid,(SELECT h.service || ':' || h.id FROM handle h WHERE h.ROWID=m.handle_id) AS capture_sender_key"
    }
    func messageRows(after: Int64, through: Int64, limit: Int = 128) throws -> [SQLiteRow] {
        try query("SELECT \(messageProjection) FROM message m WHERE m.ROWID>? AND m.ROWID<=? ORDER BY m.ROWID LIMIT ?", [String(after), String(through), String(limit)])
    }
    func recent() throws -> [SQLiteRow] { try query("SELECT \(messageProjection) FROM message m ORDER BY m.ROWID DESC LIMIT 2000") }
    func message(_ guid: String) throws -> SQLiteRow? { try query("SELECT \(messageProjection) FROM message m WHERE m.guid=?", [guid]).first }
    func reference(_ collection: String, _ guid: String) -> [String: Any] { ["$ref": ["collection": "messages." + collection, "native_key": guid]] }
    func timestamp(_ row: SQLiteRow, _ column: String) -> Any? {
        guard let raw = Double(row[column]), raw > 0 else { return nil }
        let seconds = raw > 1e12 ? raw / 1e9 : raw
        return dateFormat.string(from: Date(timeIntervalSinceReferenceDate: seconds))
    }
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
            data["emoji"] = reaction.emoji
            data["sender_handle_id"] = senderKey.map { reference("handle", $0) }
            return [("messages.reaction", guid, data)]
        }
        var data: [String: Any] = ["is_from_me": row.flag("is_from_me"), "kind": row.integer("item_type") == 0 ? "message" : "group_event", "error_code": row.integer("error"), "edit_history": [], "retracted_parts": []]
        data["chat_id"] = chat.isEmpty ? nil : reference("chat", chat)
        data["sender_handle_id"] = senderKey.map { reference("handle", $0) }
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
        if row.integer("item_type") != 0 { data["group_event"] = ["item_type": row.integer("item_type"), "action_type": row.integer("group_action_type"), "title": row["group_title"]] }
        return [("messages.message", guid, data)]
    }
    func smallObservations() throws -> [(String, String, [String: Any])] {
        var result: [(String, String, [String: Any])] = []
        for row in try query("SELECT ROWID AS row_position,* FROM handle") {
            result.append(("messages.handle", row["service"] + ":" + row["id"], ["address": row["id"], "service": row["service"]]))
        }
        for row in try query("SELECT ROWID AS row_position,* FROM chat") {
            let participants = try query("SELECT h.id,h.service FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id WHERE j.chat_id=? ORDER BY h.service,h.id", [row["row_position"]]).map { reference("handle", $0["service"] + ":" + $0["id"]) }
            var data: [String: Any] = ["service": row["service_name"], "style": row["guid"].contains(";+;") ? "group" : "direct", "display_name": row["display_name"], "participants": participants, "is_archived": row.flag("is_archived"), "capabilities": []]
            data["last_read_at"] = timestamp(row, "last_read_message_timestamp")
            result.append(("messages.chat", row["guid"], data))
        }
        return result + (try attachmentObservations())
    }
    func attachmentObservations(guid: String? = nil) throws -> [(String, String, [String: Any])] {
        var result: [(String, String, [String: Any])] = []
        for row in try query("SELECT a.ROWID AS row_position,a.*,(SELECT m.guid FROM message_attachment_join j JOIN message m ON m.ROWID=j.message_id WHERE j.attachment_id=a.ROWID ORDER BY m.ROWID LIMIT 1) AS message_guid FROM attachment a" + (guid == nil ? "" : " WHERE a.guid=?"), guid.map { [$0] } ?? []) {
            let location = attachmentURL(row["filename"])
            let exists = location.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
            let resource = try? location?.resourceValues(forKeys: [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey])
            let offloaded = (resource?.isUbiquitousItem == true && resource?.ubiquitousItemDownloadingStatus == .notDownloaded) || location.map(BlobDigest.isDataless) == true
            let state = offloaded ? "offloaded" : exists ? "present" : row.integer("transfer_state") == 0 ? "pending" : row["filename"].isEmpty ? "missing" : "offloaded"
            var blob: [String: Any] = ["state": state]
            if exists { blob["handle"] = row["guid"] }
            var data: [String: Any] = ["filename": URL(fileURLWithPath: row["filename"]).lastPathComponent, "mime_type": row["mime_type"], "uti": row["uti"], "size": row.integer("total_bytes"), "blob": blob]
            if !row["message_guid"].isEmpty { data["message_id"] = reference("message", row["message_guid"]) }
            result.append(("messages.attachment", row["guid"], data))
        }
        return result
    }
    func attachmentURL(_ path: String) -> URL? {
        guard !path.isEmpty else { return nil }
        let expanded = (path as NSString).expandingTildeInPath
        let url = URL(fileURLWithPath: expanded).standardizedFileURL.resolvingSymlinksInPath()
        let root = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/Attachments").resolvingSymlinksInPath().path + "/"
        return url.path.hasPrefix(root) ? url : nil
    }
}
