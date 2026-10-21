import Foundation
import SQLite3
import SourceSupport

final class StoreCopy {
    private var database: OpaquePointer?

    init(path: String) throws {
        var live: OpaquePointer?
        guard sqlite3_open_v2(path, &live, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
            if let live { sqlite3_close(live) }
            throw SourceFailure.permissionMissing
        }
        defer { sqlite3_close(live) }
        sqlite3_busy_timeout(live, 25)
        guard sqlite3_open(":memory:", &database) == SQLITE_OK else {
            if let database { sqlite3_close(database) }
            database = nil
            throw SourceFailure.unavailable
        }
        guard let backup = sqlite3_backup_init(database, "main", live, "main") else {
            sqlite3_close(database)
            database = nil
            throw SourceFailure.unavailable
        }
        let step = sqlite3_backup_step(backup, -1)
        let finish = sqlite3_backup_finish(backup)
        guard step == SQLITE_DONE, finish == SQLITE_OK else {
            sqlite3_close(database)
            database = nil
            throw SourceFailure.unavailable
        }
    }

    deinit { sqlite3_close(database) }

    func rows(_ sql: String) throws -> [[String: JSON]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unsupported }
        defer { sqlite3_finalize(statement) }
        var rows: [[String: JSON]] = []
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return rows }
            guard status == SQLITE_ROW else { throw SourceFailure.corrupt }
            var row: [String: JSON] = [:]
            for column in 0..<sqlite3_column_count(statement) {
                let name = String(cString: sqlite3_column_name(statement, column))
                switch sqlite3_column_type(statement, column) {
                case SQLITE_INTEGER:
                    let value = sqlite3_column_int64(statement, column)
                    row[name] = value >= 0 ? .integer(UInt64(value)) : .number(Double(value))
                case SQLITE_FLOAT: row[name] = .number(sqlite3_column_double(statement, column))
                case SQLITE_TEXT:
                    let count = Int(sqlite3_column_bytes(statement, column))
                    if let bytes = sqlite3_column_text(statement, column) {
                        row[name] = .string(String(decoding: UnsafeBufferPointer(start: bytes, count: count), as: UTF8.self))
                    }
                case SQLITE_BLOB:
                    let count = Int(sqlite3_column_bytes(statement, column))
                    if name == "ZIDENTIFIER", count == 16, let bytes = sqlite3_column_blob(statement, column) {
                        row[name] = .string(NSUUID(uuidBytes: bytes.assumingMemoryBound(to: UInt8.self)).uuidString)
                    } else {
                        row[name] = .string(sqlite3_column_blob(statement, column).map { Data(bytes: $0, count: count).base64EncodedString() } ?? "")
                    }
                default: row[name] = .null
                }
            }
            rows.append(row)
        }
    }
}
