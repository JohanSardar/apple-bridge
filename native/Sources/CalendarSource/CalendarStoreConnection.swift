import Foundation
import SQLite3
import SourceSupport

final class CalendarStoreConnection {
    private var database: OpaquePointer?

    init(path: String) throws {
        var live: OpaquePointer?
        guard sqlite3_open_v2(path, &live, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
            if let live { sqlite3_close(live) }
            throw SourceFailure.permissionMissing
        }
        defer { sqlite3_close(live) }
        sqlite3_busy_timeout(live, 25)
        guard sqlite3_open(":memory:", &database) == SQLITE_OK else { throw SourceFailure.unavailable }
        guard let backup = sqlite3_backup_init(database, "main", live, "main") else {
            sqlite3_close(database); database = nil
            throw SourceFailure.unavailable
        }
        let step = sqlite3_backup_step(backup, -1)
        let finish = sqlite3_backup_finish(backup)
        guard step == SQLITE_DONE, finish == SQLITE_OK else {
            sqlite3_close(database); database = nil
            throw SourceFailure.unavailable
        }
    }

    deinit { sqlite3_close(database) }

    func rows(_ sql: String) throws -> [[String: JSON]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unsupported }
        defer { sqlite3_finalize(statement) }
        var result: [[String: JSON]] = []
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return result }
            guard status == SQLITE_ROW else { throw SourceFailure.corrupt }
            var row: [String: JSON] = [:]
            for index in 0..<sqlite3_column_count(statement) {
                let key = String(cString: sqlite3_column_name(statement, index))
                switch sqlite3_column_type(statement, index) {
                case SQLITE_TEXT:
                    guard let text = sqlite3_column_text(statement, index) else { throw SourceFailure.corrupt }
                    row[key] = .string(String(decoding: UnsafeBufferPointer(start: text, count: Int(sqlite3_column_bytes(statement, index))), as: UTF8.self))
                case SQLITE_INTEGER, SQLITE_FLOAT:
                    let value = sqlite3_column_double(statement, index)
                    row[key] = value.isFinite ? .number(value) : .null
                default: row[key] = .null
                }
            }
            result.append(row)
        }
    }
}
