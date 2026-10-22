import Foundation
import SQLite3
import CryptoKit
import SourceSupport
import Decoders

struct NotesSnapshot {
    let generation: String
    let observations: [JSON]
    let attachments: [String: NotesAttachment]
    let rawBodies: [String: Data]
}

struct NotesStore {
    let path: String
    func read() throws -> NotesSnapshot {
        var source: OpaquePointer?
        guard sqlite3_open_v2(path, &source, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if source != nil { sqlite3_close(source) }; throw SourceFailure.permissionMissing }
        defer { sqlite3_close(source) }
        sqlite3_busy_timeout(source, 1000)
        var copy: OpaquePointer?
        guard sqlite3_open(":memory:", &copy) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_close(copy) }
        guard let backup = sqlite3_backup_init(copy, "main", source, "main") else { throw SourceFailure.unavailable }
        let status = sqlite3_backup_step(backup, -1), finish = sqlite3_backup_finish(backup)
        guard status == SQLITE_DONE, finish == SQLITE_OK else { throw SourceFailure.unavailable }
        func query(_ sql: String, _ values: [String] = []) throws -> [[String: JSON]] {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(copy, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
            defer { sqlite3_finalize(statement) }
            for (index, value) in values.enumerated() {
                let result = value.withCString { sqlite3_bind_text(statement, Int32(index + 1), $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
                guard result == SQLITE_OK else { throw SourceFailure.unavailable }
            }
            var result: [[String: JSON]] = []
            while true {
                let step = sqlite3_step(statement)
                if step == SQLITE_DONE { return result }
                guard step == SQLITE_ROW else { throw SourceFailure.unavailable }
                var row: [String: JSON] = [:]
                for column in 0..<sqlite3_column_count(statement) {
                    let name = String(cString: sqlite3_column_name(statement, column))
                    switch sqlite3_column_type(statement, column) {
                    case SQLITE_INTEGER, SQLITE_FLOAT: row[name] = .number(sqlite3_column_double(statement, column))
                    case SQLITE_TEXT: row[name] = .string(String(cString: sqlite3_column_text(statement, column)))
                    case SQLITE_BLOB:
                        if let bytes = sqlite3_column_blob(statement, column) { row[name] = .string(Data(bytes: bytes, count: Int(sqlite3_column_bytes(statement, column))).base64EncodedString()) }
                    default: break
                    }
                }
                result.append(row)
            }
        }
        guard let uuid = try query("SELECT Z_UUID FROM Z_METADATA").first?["Z_UUID"]?.string, !uuid.isEmpty else { throw SourceFailure.corrupt }
        let entities = try query("SELECT Z_ENT,Z_NAME FROM Z_PRIMARYKEY")
        guard let noteEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICNote" })?["Z_ENT"]?.number,
              let folderEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICFolder" })?["Z_ENT"]?.number,
              try query("PRAGMA table_info(ZICCLOUDSYNCINGOBJECT)").contains(where: { $0["name"]?.string == "ZISPASSWORDPROTECTED" }) else { throw SourceFailure.unsupported }
        let objects = try query("SELECT o.*,d.ZDATA AS BODY FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICNOTEDATA d ON d.Z_PK=o.ZNOTEDATA WHERE o.Z_ENT IN (\(Int(noteEntity)),\(Int(folderEntity))) AND COALESCE(o.ZMARKEDFORDELETION,0)=0 ORDER BY o.Z_ENT DESC,o.Z_PK")
        let accountRows = try query("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICAccount') AND COALESCE(ZMARKEDFORDELETION,0)=0")
        let accounts = Dictionary(uniqueKeysWithValues: accountRows.map { (Int($0["Z_PK"]?.number ?? 0), $0) })
        let folderRows = objects.filter { $0["Z_ENT"]?.number == folderEntity }
        let folders = Dictionary(uniqueKeysWithValues: folderRows.map { (Int($0["Z_PK"]?.number ?? 0), $0) })
        func folderFlags(_ row: [String: JSON]) -> (recent: Bool, readOnly: Bool) {
            var current = row, seen = Set<Int>(), recent = false, readOnly = false
            for _ in 0..<128 {
                let key = Int(current["Z_PK"]?.number ?? 0)
                guard seen.insert(key).inserted else { return (recent, true) }
                recent = recent || current["ZFOLDERTYPE"]?.number == 1
                readOnly = readOnly || recent || current["ZSMARTFOLDERQUERYJSON"] != nil
                let parent = Int(current["ZPARENT"]?.number ?? 0)
                guard parent > 0 else { return (recent, readOnly) }
                guard let next = folders[parent] else { return (recent, true) }
                current = next
            }
            return (recent, true)
        }
        func account(_ row: [String: JSON]) -> [String: JSON] {
            if let id = row["ZACCOUNT8"]?.number, let value = accounts[Int(id)] { return value }
            let candidates = row.filter { $0.key.hasPrefix("ZACCOUNT") }.compactMap { accounts[Int($0.value.number)] }
            return candidates.count == 1 ? candidates[0] : [:]
        }
        func ref(_ key: String) -> JSON { .object(["$ref": .object(["collection": .string("notes.folder"), "native_key": .string(key)])]) }
        func observation(_ collection: String, _ row: [String: JSON], _ data: [String: JSON]) throws -> JSON {
            guard let key = row["ZIDENTIFIER"]?.string, !key.isEmpty else { throw SourceFailure.corrupt }
            let kind = collection == "notes.folder" ? "ICFolder" : "ICNote"
            let nativeID = "x-coredata://" + uuid + "/" + kind + "/p" + String(Int(row["Z_PK"]?.number ?? 0))
            return .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(nativeID), "data": .object(data), "observed_at": .string(timestamp())])
        }
        var observations: [JSON] = [], attachments: [String: NotesAttachment] = [:], rawBodies: [String: Data] = [:]
        for row in folderRows {
            let owner = account(row)
            let flags = folderFlags(row), recent = flags.recent
            let smart = row["ZSMARTFOLDERQUERYJSON"] != nil
            var data: [String: JSON] = ["title": row["ZTITLE2"] ?? .string(""), "account": owner["ZIDENTIFIER"] ?? .string(""), "recently_deleted": .bool(recent), "read_only": .bool(flags.readOnly)]
            if let name = owner["ZNAME"] { data["account_name"] = name }
            if smart { data["smart"] = .bool(true); data["smart_filter_status"] = .string("unsupported") }
            if let parent = folders[Int(row["ZPARENT"]?.number ?? 0)]?["ZIDENTIFIER"]?.string { data["parent_id"] = ref(parent) }
            observations.append(try observation("notes.folder", row, data))
        }
        for row in objects where row["Z_ENT"]?.number == noteEntity {
            guard let folder = folders[Int(row["ZFOLDER"]?.number ?? 0)], let folderKey = folder["ZIDENTIFIER"]?.string, !folderKey.isEmpty,
                  let key = row["ZIDENTIFIER"]?.string, !key.isEmpty else { continue }
            let locked = (row["ZISPASSWORDPROTECTED"]?.number ?? 0) != 0
            let flags = folderFlags(folder), recent = flags.recent
            let owner = account(folder)
            var data: [String: JSON] = ["folder_id": ref(folderKey), "title": row["ZTITLE1"] ?? .string(""), "locked": .bool(locked), "plain_text": .string(""), "attachments": .array([]), "content_status": .string(locked ? "locked" : "offloaded"), "editable": .string("none"), "recently_deleted": .bool(recent), "read_only": .bool(flags.readOnly)]
            for (native, field) in [("ZCREATIONDATE1", "created_at"), ("ZMODIFICATIONDATE1", "modified_at")] {
                if let value = row[native]?.number { data[field] = .string(timestamp(Date(timeIntervalSinceReferenceDate: value))) }
            }
            if let pinned = row["ZISPINNED"]?.number, pinned == 0 || pinned == 1 { data["pinned"] = .bool(pinned == 1) }
            var bodyDigest = "", stamps: [String: JSON] = [:]
            if !locked, let bytes = Data(base64Encoded: row["BODY"]?.string ?? ""), !bytes.isEmpty {
                bodyDigest = digest(bytes)
                do {
                    var decoded = try NotesProto.decode(bytes)
                    var known: [String: [String: JSON]] = [:], tables: [String: Data] = [:]
                    for _ in 0..<8 {
                        let missing = decoded.attachments.filter { known[$0] == nil }
                        if missing.isEmpty { break }
                        for start in stride(from: 0, to: missing.count, by: 128) {
                            let ids = Array(missing[start..<min(start + 128, missing.count)])
                            let values = try query("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE ZIDENTIFIER IN (" + Array(repeating: "?", count: ids.count).joined(separator: ",") + ")", ids)
                            for value in values {
                                let id = value["ZIDENTIFIER"]?.string ?? ""
                                guard NotesAttachment.belongs(value, note: Int(row["Z_PK"]?.number ?? 0), query: query) else { continue }
                                known[id] = value
                                for field in ["ZMERGEABLEDATA1", "ZMERGEABLEDATA"] {
                                    if let encoded = value[field]?.string, let table = Data(base64Encoded: encoded) { tables[id] = table; break }
                                }
                            }
                        }
                        decoded = try NotesProto.decode(bytes, tables: tables)
                        if missing.allSatisfy({ known[$0] == nil }) { break }
                    }
                    data["plain_text"] = .string(decoded.text)
                    data["blocks"] = try JSONDecoder().decode(JSON.self, from: JSONEncoder().encode(decoded.blocks))
                    data["html"] = .string(decoded.html)
                    data["content_status"] = .string("complete")
                    data["block_status"] = .string(decoded.blocks.contains(where: NotesAttachment.unsupported) ? "partial" : "complete")
                    data["editable"] = .string(!flags.readOnly && !decoded.rich && decoded.lossless ? "plain" : "none")
                    data["round_trip_status"] = .string(decoded.lossless ? "unproven" : "unsupported")
                    var values: [JSON] = [], tags = Set<String>()
                    for id in decoded.attachments {
                        guard let value = known[id] else {
                            values.append(.object(["id": .string(id), "state": .string("unavailable"), "reason": .string("attachment_metadata_unavailable")]))
                            continue
                        }
                        let asset = try NotesAttachment(row: value, account: owner["ZIDENTIFIER"]?.string ?? "", root: URL(fileURLWithPath: path).deletingLastPathComponent(), query: query)
                        attachments[key + ":" + id] = asset
                        values.append(asset.metadata)
                        stamps[id] = .string(asset.stamp)
                        if asset.uti == "com.apple.notes.inlinetextattachment.hashtag", asset.alternateText.hasPrefix("#") { tags.insert(asset.alternateText) }
                    }
                    data["attachments"] = .array(values)
                    data["tags"] = .array(tags.sorted().map(JSON.string))
                } catch {
                    for field in ["plain_text", "html", "blocks", "tags", "block_status", "round_trip_status"] { data.removeValue(forKey: field) }
                    data["plain_text"] = .string("")
                    data["attachments"] = .array([])
                    data["editable"] = .string("none")
                    data["content_status"] = .string("undecoded")
                    data["raw_body_digest"] = .string(bodyDigest)
                    rawBodies[key] = bytes
                }
            }
            var fingerprint = data
            fingerprint["body_digest"] = .string(bodyDigest)
            fingerprint["media_stamps"] = .object(stamps)
            let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
            data["native_fingerprint"] = .string(digest(try encoder.encode(JSON.object(fingerprint))))
            observations.append(try observation("notes.note", row, data))
        }
        return NotesSnapshot(generation: "notes-" + uuid, observations: observations, attachments: attachments, rawBodies: rawBodies)
    }
    private func digest(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
}
