import Foundation
import SQLite3
import SourceSupport

final class NotesDatabase {
    private var connection: OpaquePointer?
    init(path: String) throws {
        guard sqlite3_open_v2(path, &connection, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if connection != nil { sqlite3_close(connection) }; connection = nil
            throw SourceFailure.permissionMissing
        }
        sqlite3_busy_timeout(connection, 1000)
        do { try execute("BEGIN") } catch { sqlite3_close(connection); connection = nil; throw error }
    }
    deinit { sqlite3_exec(connection, "ROLLBACK", nil, nil, nil); sqlite3_close(connection) }
    private func execute(_ sql: String) throws {
        guard sqlite3_exec(connection, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
    }
    func query(_ sql: String, _ values: [String] = []) throws -> [[String: JSON]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
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
    func columns(_ table: String) throws -> Set<String> { Set(try query("PRAGMA table_info(" + table + ")").map { $0["name"]?.string ?? "" }) }
}
