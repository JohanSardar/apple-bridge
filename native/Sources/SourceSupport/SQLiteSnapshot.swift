import Foundation
import SQLite3
import CryptoKit
import Darwin

public enum SQLiteValue: Sendable, Equatable {
    case integer(Int64), real(Double), text(String), blob(Data), null
    public var text: String? { if case .text(let value) = self { return value }; return nil }
}

public struct SQLiteRows: Sendable {
    public let rows: [[String: SQLiteValue]]
    public let complete: Bool
}

public final class SQLiteSnapshot {
    private var connection: OpaquePointer?
    private let path: String
    private let generationSQL: String
    private var identity = ""
    public private(set) var generation = ""

    public init(path: String, generationSQL: String) throws {
        self.path = path
        self.generationSQL = generationSQL
        identity = try Self.fileIdentity(path)
        guard sqlite3_open_v2(path, &connection, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            if let connection { sqlite3_close(connection) }
            connection = nil
            throw SourceFailure.unavailable
        }
        sqlite3_busy_timeout(connection, 250)
        do {
            guard sqlite3_exec(connection, "BEGIN", nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
            let metadata = try query(generationSQL)
            guard metadata.complete, metadata.rows.count == 1, metadata.rows[0].count == 1,
                  let value = metadata.rows[0].values.first?.text, !value.isEmpty else { throw SourceFailure.unsupported }
            let schema = try query("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
            let version = try query("PRAGMA schema_version")
            guard schema.complete, version.complete else { throw SourceFailure.unavailable }
            let material = identity + "|" + value + "|" + String(describing: version.rows) + "|" + schema.rows.map { row in row.keys.sorted().map { $0 + "=" + String(describing: row[$0]!) }.joined(separator: "|") }.joined(separator: "\n")
            generation = SHA256.hash(data: Data(material.utf8)).map { String(format: "%02x", $0) }.joined()
            guard identity == (try Self.fileIdentity(path)) else { throw SourceFailure.conflict }
        } catch {
            sqlite3_close(connection)
            connection = nil
            throw error
        }
    }

    deinit {
        if let connection {
            sqlite3_exec(connection, "ROLLBACK", nil, nil, nil)
            sqlite3_close(connection)
        }
    }

    public func isCurrent() throws -> Bool {
        guard identity == (try Self.fileIdentity(path)) else { return false }
        return try SQLiteSnapshot(path: path, generationSQL: generationSQL).generation == generation
    }

    public func requireSchema(_ required: [String: Set<String>]) throws {
        for (table, columns) in required {
            let quoted = table.replacingOccurrences(of: "\"", with: "\"\"")
            let found = try query("PRAGMA table_info(\"" + quoted + "\")")
            guard found.complete, !found.rows.isEmpty, columns.isSubset(of: Set(found.rows.compactMap { $0["name"]?.text })) else { throw SourceFailure.unsupported }
        }
    }

    public func query(_ sql: String, values: [String] = [], maxRows: Int = 100_000, timeout: TimeInterval = 5) throws -> SQLiteRows {
        guard maxRows >= 0, timeout.isFinite, timeout >= 0 else { throw SourceFailure.invalid }
        let deadline = ProcessInfo.processInfo.systemUptime + timeout
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, sql, -1, &statement, nil) == SQLITE_OK, let statement else { throw SourceFailure.unsupported }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_stmt_readonly(statement) != 0, sqlite3_bind_parameter_count(statement) == values.count else { throw SourceFailure.invalid }
        for (index, value) in values.enumerated() {
            let status = value.withCString { sqlite3_bind_text(statement, Int32(index + 1), $0, Int32(value.utf8.count), unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard status == SQLITE_OK else { throw SourceFailure.invalid }
        }
        let bound = UnsafeMutablePointer<Double>.allocate(capacity: 1)
        bound.initialize(to: deadline)
        defer { sqlite3_progress_handler(connection, 0, nil, nil); bound.deinitialize(count: 1); bound.deallocate() }
        sqlite3_progress_handler(connection, 1000, { pointer in
            guard let pointer else { return 1 }
            return ProcessInfo.processInfo.systemUptime >= pointer.assumingMemoryBound(to: Double.self).pointee ? 1 : 0
        }, bound)
        var rows: [[String: SQLiteValue]] = []
        while true {
            try Task.checkCancellation()
            guard ProcessInfo.processInfo.systemUptime < deadline else { return SQLiteRows(rows: rows, complete: false) }
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return SQLiteRows(rows: rows, complete: true) }
            if status == SQLITE_INTERRUPT { return SQLiteRows(rows: rows, complete: false) }
            guard status == SQLITE_ROW else { throw SourceFailure.unavailable }
            guard rows.count < maxRows else { return SQLiteRows(rows: rows, complete: false) }
            var row: [String: SQLiteValue] = [:]
            for column in 0..<sqlite3_column_count(statement) {
                let name = String(cString: sqlite3_column_name(statement, column))
                switch sqlite3_column_type(statement, column) {
                case SQLITE_INTEGER: row[name] = .integer(sqlite3_column_int64(statement, column))
                case SQLITE_FLOAT: row[name] = .real(sqlite3_column_double(statement, column))
                case SQLITE_TEXT:
                    let count = Int(sqlite3_column_bytes(statement, column))
                    guard let bytes = sqlite3_column_text(statement, column),
                          let text = String(data: Data(bytes: bytes, count: count), encoding: .utf8) else { throw SourceFailure.corrupt }
                    row[name] = .text(text)
                case SQLITE_BLOB:
                    let count = Int(sqlite3_column_bytes(statement, column))
                    row[name] = .blob(sqlite3_column_blob(statement, column).map { Data(bytes: $0, count: count) } ?? Data())
                default: row[name] = .null
                }
            }
            rows.append(row)
        }
    }

    private static func fileIdentity(_ path: String) throws -> String {
        var value = stat()
        guard stat(path, &value) == 0 else { throw SourceFailure.unavailable }
        return "\(value.st_dev):\(value.st_ino):\(value.st_birthtimespec.tv_sec):\(value.st_birthtimespec.tv_nsec)"
    }
}
