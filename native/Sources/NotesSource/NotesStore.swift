import Foundation
import SQLite3
import CryptoKit
import SourceSupport
import Decoders

struct NotesSnapshot {
    let generation: String
    let observations: [JSON]
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
        let status = sqlite3_backup_step(backup, -1)
        let finish = sqlite3_backup_finish(backup)
        guard status == SQLITE_DONE, finish == SQLITE_OK else { throw SourceFailure.unavailable }
        func query(_ sql: String) throws -> [[String: JSON]] {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(copy, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
            defer { sqlite3_finalize(statement) }
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
        guard let noteEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICNote" })?["Z_ENT"]?.number, let folderEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICFolder" })?["Z_ENT"]?.number else { throw SourceFailure.unsupported }
        let objects = try query("SELECT o.*, d.ZDATA AS BODY FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICNOTEDATA d ON d.Z_PK=o.ZNOTEDATA WHERE o.Z_ENT IN (\(Int(noteEntity)),\(Int(folderEntity))) AND COALESCE(o.ZMARKEDFORDELETION,0)=0 AND NOT (o.Z_ENT=\(Int(folderEntity)) AND COALESCE(o.ZFOLDERTYPE,0)=1) AND NOT (o.Z_ENT=\(Int(noteEntity)) AND o.ZFOLDER IN (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=\(Int(folderEntity)) AND COALESCE(ZFOLDERTYPE,0)=1)) ORDER BY o.Z_ENT DESC,o.Z_PK")
        let accounts = Dictionary(uniqueKeysWithValues: try query("SELECT Z_PK,ZIDENTIFIER FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICAccount')").map { (Int($0["Z_PK"]?.number ?? 0), $0["ZIDENTIFIER"] ?? .string("")) })
        let folders = Dictionary(uniqueKeysWithValues: objects.filter { $0["Z_ENT"]?.number == folderEntity }.map { (Int($0["Z_PK"]?.number ?? 0), $0["ZIDENTIFIER"]?.string ?? "") })
        func ref(_ key: String) -> JSON { .object(["$ref": .object(["collection": .string("notes.folder"), "native_key": .string(key)])]) }
        var observations: [JSON] = []
        for row in objects {
            let key = row["ZIDENTIFIER"]?.string ?? ""
            guard !key.isEmpty else { throw SourceFailure.corrupt }
            let isFolder = row["Z_ENT"]?.number == folderEntity
            let nativeID = "x-coredata://" + uuid + (isFolder ? "/ICFolder/p" : "/ICNote/p") + String(Int(row["Z_PK"]?.number ?? 0))
            var data: [String: JSON] = [:]
            if isFolder {
                data = ["title": row["ZTITLE2"] ?? .string(""), "account": accounts[Int(row["ZACCOUNT8"]?.number ?? 0)] ?? .string("")]
                if let parent = folders[Int(row["ZPARENT"]?.number ?? 0)] { data["parent_id"] = ref(parent) }
            } else {
                let locked = (row["ZISPASSWORDPROTECTED"]?.number ?? 0) != 0
                data = ["folder_id": ref(folders[Int(row["ZFOLDER"]?.number ?? 0)] ?? ""), "title": row["ZTITLE1"] ?? .string(""), "locked": .bool(locked), "plain_text": .string(""), "attachments": .array([]), "content_status": .string(locked ? "locked" : "offloaded"), "editable": .string("none")]
                for (native, field) in [("ZCREATIONDATE1", "created_at"), ("ZMODIFICATIONDATE1", "modified_at")] {
                    data[field] = .string(timestamp(Date(timeIntervalSinceReferenceDate: row[native]?.number ?? 0)))
                }
                if !locked, let bytes = Data(base64Encoded: row["BODY"]?.string ?? ""), !bytes.isEmpty {
                    if let decoded = try? NotesProto.decode(bytes) {
                        let rich = decoded.rich || (row["ZHASCHECKLIST"]?.number ?? 0) != 0 || (row["ZHASSYSTEMTEXTATTACHMENTS"]?.number ?? 0) != 0
                        data["plain_text"] = .string(decoded.text)
                        data["content_status"] = .string("complete")
                        data["editable"] = .string(rich ? "none" : "plain")
                        data["attachments"] = .array(decoded.attachments.map(JSON.string))
                    } else {
                        data["content_status"] = .string("undecoded")
                        data["raw_body_digest"] = .string(SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined())
                    }
                }
            }
            observations.append(.object(["kind": .string("upsert"), "collection": .string(isFolder ? "notes.folder" : "notes.note"), "native_key": .string(key), "scoped_alias": .string(nativeID), "data": .object(data), "observed_at": .string(timestamp())]))
        }
        return NotesSnapshot(generation: "notes-" + uuid, observations: observations)
    }
}
