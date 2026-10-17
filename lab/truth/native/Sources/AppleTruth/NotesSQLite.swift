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
        func rows(_ sql: String) throws -> [[String: Any]] {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw NSError(domain: "NotesSQLiteQuery", code: Int(sqlite3_errcode(database)), userInfo: [NSLocalizedDescriptionKey: String(cString: sqlite3_errmsg(database))]) }
            defer { sqlite3_finalize(statement) }
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
        let folders = try rows("SELECT ZIDENTIFIER AS id,ZTITLE2 AS title,Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=\(folder) AND COALESCE(ZMARKEDFORDELETION,0)=0 AND COALESCE(ZFOLDERTYPE,0)<>1")
        let folderKeys = Dictionary(uniqueKeysWithValues: folders.map { ($0["Z_PK"] as? Int64 ?? 0, $0["id"] as? String ?? "") })
        let objects = try rows("SELECT o.ZIDENTIFIER AS id,o.ZTITLE1 AS title,o.ZISPASSWORDPROTECTED AS locked,o.ZFOLDER,d.ZDATA FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICNOTEDATA d ON d.Z_PK=o.ZNOTEDATA WHERE o.Z_ENT=\(note) AND COALESCE(o.ZMARKEDFORDELETION,0)=0 AND o.ZFOLDER NOT IN (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=\(folder) AND COALESCE(ZFOLDERTYPE,0)=1)")
        var notes: [[String: Any]] = []
        for object in objects {
            let locked = (object["locked"] as? Int64 ?? 0) != 0
            var row: [String: Any] = ["id":object["id"] as? String ?? "", "title":object["title"] as? String ?? "", "locked":locked, "plain_text":"", "content_status":locked ? "locked" : "offloaded", "folder_id":folderKeys[object["ZFOLDER"] as? Int64 ?? 0] ?? ""]
            if !locked, let compressed = object["ZDATA"] as? Data, !compressed.isEmpty {
                if let text = try? truthNoteText(compressed) { row["plain_text"] = text; row["content_status"] = "complete" }
                else { row["content_status"] = "undecoded" }
            }
            notes.append(row)
        }
        return ["status":"verified", "records":notes, "folders":folders.map { ["id":$0["id"] as? String ?? "", "title":$0["title"] as? String ?? ""] }]
    }
    private static func truthNoteText(_ input: Data) throws -> String {
        var raw = input
        if input.starts(with: [31,139]) {
            var state = z_stream()
            guard inflateInit2_(&state, 31, ZLIB_VERSION, Int32(MemoryLayout<z_stream>.size)) == Z_OK else { throw NotesSQLiteFailure.decode }
            defer { inflateEnd(&state) }
            raw = try input.withUnsafeBytes { bytes in
                state.next_in = UnsafeMutablePointer(mutating: bytes.bindMemory(to: Bytef.self).baseAddress); state.avail_in = uInt(bytes.count)
                var result = Data(), block = [UInt8](repeating: 0, count: 8192)
                while true {
                    let code = block.withUnsafeMutableBytes { destination in
                        state.next_out = destination.bindMemory(to: Bytef.self).baseAddress; state.avail_out = uInt(destination.count)
                        return inflate(&state, Z_NO_FLUSH)
                    }
                    result.append(contentsOf: block.prefix(block.count - Int(state.avail_out)))
                    guard result.count < 32 * 1024 * 1024 else { throw NotesSQLiteFailure.decode }
                    if code == Z_STREAM_END { return result }
                    guard code == Z_OK else { throw NotesSQLiteFailure.decode }
                }
            }
        }
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
        guard let text = String(data: try child(child(child(raw, number: 2), number: 3), number: 2), encoding: .utf8) else { throw NotesSQLiteFailure.decode }
        return text
    }
}

private enum NotesSQLiteFailure: Error { case read, decode, open, memory, backup, schema }
