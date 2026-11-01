import Foundation
import SQLite3
import zlib

extension AppleTruth {
    static func notesSQLite(path supplied: String?) throws -> [String: Any] {
        let path = supplied ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path
        var original: OpaquePointer?
        guard sqlite3_open_v2(path, &original, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if original != nil { sqlite3_close(original) }; throw NotesSQLiteFailure.open }
        defer { sqlite3_close(original) }
        var database: OpaquePointer?
        guard sqlite3_open(":memory:", &database) == SQLITE_OK else { throw NotesSQLiteFailure.memory }
        defer { sqlite3_close(database) }
        guard let backup = sqlite3_backup_init(database, "main", original, "main") else { throw NSError(domain: "NotesBackupInit", code: Int(sqlite3_errcode(database))) }
        let status = sqlite3_backup_step(backup, -1), finished = sqlite3_backup_finish(backup)
        guard status == SQLITE_DONE, finished == SQLITE_OK else { throw NSError(domain: "NotesBackupStep", code: Int(status == SQLITE_DONE ? finished : status)) }
        func rows(_ sql: String, _ bindings: [String] = []) throws -> [[String: Any]] {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw NSError(domain: "NotesSQLiteQuery", code: Int(sqlite3_errcode(database)), userInfo: [NSLocalizedDescriptionKey: String(cString: sqlite3_errmsg(database))]) }
            defer { sqlite3_finalize(statement) }
            for (index, value) in bindings.enumerated() {
                let text = value as NSString
                guard sqlite3_bind_text(statement, Int32(index + 1), text.utf8String, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) == SQLITE_OK else { throw NotesSQLiteFailure.read }
                withExtendedLifetime(text) {}
            }
            var output: [[String: Any]] = []
            while true {
                let result = sqlite3_step(statement)
                if result == SQLITE_DONE { return output }
                guard result == SQLITE_ROW else { throw NotesSQLiteFailure.read }
                var row: [String: Any] = [:]
                for index in 0..<sqlite3_column_count(statement) {
                    let column = String(cString: sqlite3_column_name(statement, index))
                    switch sqlite3_column_type(statement, index) {
                    case SQLITE_INTEGER: row[column] = sqlite3_column_int64(statement, index)
                    case SQLITE_TEXT: row[column] = String(cString: sqlite3_column_text(statement, index))
                    case SQLITE_BLOB:
                        if let pointer = sqlite3_column_blob(statement, index) { row[column] = Data(bytes: pointer, count: Int(sqlite3_column_bytes(statement, index))) }
                    default: break
                    }
                }
                output.append(row)
            }
        }
        let entities = try rows("SELECT Z_NAME,Z_ENT FROM Z_PRIMARYKEY")
        guard let note = entities.first(where: { $0["Z_NAME"] as? String == "ICNote" })?["Z_ENT"] as? Int64, let folder = entities.first(where: { $0["Z_NAME"] as? String == "ICFolder" })?["Z_ENT"] as? Int64 else { throw NotesSQLiteFailure.schema }
        let folders = try rows("SELECT ZIDENTIFIER AS id,ZTITLE2 AS title,Z_PK,ZFOLDERTYPE FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=\(folder) AND COALESCE(ZMARKEDFORDELETION,0)=0")
        let folderKeys = Dictionary(uniqueKeysWithValues: folders.map { ($0["Z_PK"] as? Int64 ?? 0, $0["id"] as? String ?? "") })
        let columns = Set(try rows("PRAGMA table_info(ZICCLOUDSYNCINGOBJECT)").compactMap { $0["name"] as? String })
        let owners = ["ZNOTE", "ZNOTE1"].filter { columns.contains($0) }
        let tableColumn = ["ZMERGEABLEDATA1", "ZMERGEABLEDATA"].first { columns.contains($0) }
        let objects = try rows("WITH RECURSIVE excluded(pk) AS (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=\(folder) AND (ZFOLDERTYPE=1 OR COALESCE(ZMARKEDFORDELETION,0)<>0) UNION SELECT f.Z_PK FROM ZICCLOUDSYNCINGOBJECT f JOIN excluded p ON f.ZPARENT=p.pk WHERE f.Z_ENT=\(folder)) SELECT o.Z_PK AS native_pk,o.ZIDENTIFIER AS id,o.ZTITLE1 AS title,o.ZISPASSWORDPROTECTED AS locked,o.ZFOLDER,d.ZDATA FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICNOTEDATA d ON d.Z_PK=o.ZNOTEDATA WHERE o.Z_ENT=\(note) AND COALESCE(o.ZMARKEDFORDELETION,0)=0 AND o.ZFOLDER NOT IN (SELECT pk FROM excluded)")
        var notes: [[String: Any]] = []
        for object in objects {
            guard let folderKey = folderKeys[object["ZFOLDER"] as? Int64 ?? 0], !folderKey.isEmpty else { continue }
            let locked = (object["locked"] as? Int64 ?? 0) != 0
            var row: [String: Any] = ["id":object["id"] as? String ?? "", "title":object["title"] as? String ?? "", "locked":locked, "plain_text":"", "content_status":locked ? "locked" : "offloaded", "folder_id":folderKey]
            if !locked, let compressed = object["ZDATA"] as? Data, !compressed.isEmpty {
                if let decoded = try? truthNoteText(compressed) {
                    row["plain_text"] = decoded.0; row["content_status"] = "complete"
                    var counts = decoded.1, cells = 0, complete = true
                    for id in decoded.2 {
                        guard let tableColumn, !owners.isEmpty, let owner = object["native_pk"] as? Int64 else { complete = false; break }
                        let condition = owners.map { $0 + "=?" }.joined(separator: " OR ")
                        let tableRows = try rows("SELECT \(tableColumn) AS bytes FROM ZICCLOUDSYNCINGOBJECT WHERE ZIDENTIFIER=? AND (\(condition))", [id] + owners.map { _ in String(owner) })
                        guard tableRows.count == 1, let bytes = tableRows[0]["bytes"] as? Data, let count = try? NotesTruthStructure.tableCells(bytes) else { complete = false; break }
                        cells += count
                    }
                    if complete { counts["table_cells"] = cells }
                    row["structure_counts"] = counts
                    row["table_cells_status"] = complete ? "complete" : "unavailable"
                }
                else { row["content_status"] = "undecoded" }
            }
            notes.append(row)
        }
        return ["status":"verified", "records":notes, "folders":folders.map { ["id":$0["id"] as? String ?? "", "title":$0["title"] as? String ?? "", "recently_deleted":($0["ZFOLDERTYPE"] as? Int64 ?? 0) == 1] as [String: Any] }]
    }
    private static func truthNoteText(_ input: Data) throws -> (String, [String: Int], [String]) {
        let raw = try NotesTruthStructure.unpack(input)
        func child(_ data: Data, number: Int) throws -> Data {
            let bytes = Array(data)
            var position = 0
            func unsigned() throws -> UInt64 {
                var number: UInt64 = 0, shift = 0
                while position < bytes.count && shift <= 63 {
                    let byte = bytes[position]; position += 1
                    guard shift < 63 || byte < 2 else { throw NotesSQLiteFailure.decode }
                    number |= UInt64(byte & 127) << shift
                    if byte & 128 == 0 { return number }
                    shift += 7
                }
                throw NotesSQLiteFailure.decode
            }
            while position < bytes.count {
                let key = try unsigned()
                var size: UInt64 = 0
                switch key & 7 {
                case 0: _ = try unsigned(); continue
                case 1: size = 8
                case 5: size = 4
                case 2: size = try unsigned()
                default: throw NotesSQLiteFailure.decode
                }
                guard size <= UInt64(bytes.count-position) else { throw NotesSQLiteFailure.decode }
                let content = Data(bytes[position..<position+Int(size)]); position += Int(size)
                if key & 7 == 2 && key >> 3 == UInt64(number) { return content }
            }
            throw NotesSQLiteFailure.decode
        }
        let note = try child(child(raw, number: 2), number: 3)
        guard let text = String(data: try child(note, number: 2), encoding: .utf8) else { throw NotesSQLiteFailure.decode }
        let counts = try NotesTruthStructure.count(note)
        return (text, counts, try NotesTruthStructure.tableIDs(note))
    }
}

private enum NotesSQLiteFailure: Error { case read, decode, open, memory, backup, schema }
