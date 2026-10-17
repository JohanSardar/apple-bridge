import Foundation
import SQLite3
import Decoders

final class MessagesBaseline {
    private var connection: OpaquePointer?
    init(path: String) throws {
        let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridge/messages", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let url = directory.appendingPathComponent(BodyDigest.hash(Data(path.utf8)) + ".sqlite")
        if !FileManager.default.fileExists(atPath: url.path) { _ = FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600]) }
        guard sqlite3_open_v2(url.path, &connection, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        try execute("PRAGMA journal_mode=WAL")
        try execute("PRAGMA synchronous=FULL")
        try execute("CREATE TABLE IF NOT EXISTS identities(identity TEXT PRIMARY KEY) WITHOUT ROWID")
        try execute("CREATE TABLE IF NOT EXISTS blobs(guid TEXT PRIMARY KEY, size INTEGER, modified REAL, digest TEXT) WITHOUT ROWID")
    }
    deinit { sqlite3_close(connection) }
    private func execute(_ sql: String, _ binding: String? = nil) throws {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, sql, -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        if let binding {
            let result = binding.withCString { sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard result == SQLITE_OK else { throw ProbeFailure.unavailable }
        }
        var result = sqlite3_step(statement)
        while result == SQLITE_ROW { result = sqlite3_step(statement) }
        guard result == SQLITE_DONE else { throw ProbeFailure.unavailable }
    }
    func identities() throws -> Set<String> {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, "SELECT identity FROM identities", -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        var result: Set<String> = []
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return result }
            guard status == SQLITE_ROW, let value = sqlite3_column_text(statement, 0) else { throw ProbeFailure.unavailable }
            result.insert(String(cString: value))
        }
    }
    func blobs() throws -> [String: (Int64, Date, String)] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, "SELECT guid,size,modified,digest FROM blobs", -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        var result: [String: (Int64, Date, String)] = [:]
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return result }
            guard status == SQLITE_ROW, let key = sqlite3_column_text(statement, 0), let digest = sqlite3_column_text(statement, 3) else { throw ProbeFailure.unavailable }
            result[String(cString: key)] = (sqlite3_column_int64(statement, 1), Date(timeIntervalSince1970: sqlite3_column_double(statement, 2)), String(cString: digest))
        }
    }
    func saveBlob(_ key: String, _ value: (Int64, Date, String)) throws {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, "INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)", -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        let keyStatus = key.withCString { sqlite3_bind_text(statement, 1, $0, -1, transient) }
        let digestStatus = value.2.withCString { sqlite3_bind_text(statement, 4, $0, -1, transient) }
        guard keyStatus == SQLITE_OK, digestStatus == SQLITE_OK, sqlite3_bind_int64(statement, 2, value.0) == SQLITE_OK,
              sqlite3_bind_double(statement, 3, value.1.timeIntervalSince1970) == SQLITE_OK, sqlite3_step(statement) == SQLITE_DONE else { throw ProbeFailure.unavailable }
    }
    func update(upserts: [String] = [], absent: [String] = []) throws {
        guard !upserts.isEmpty || !absent.isEmpty else { return }
        try execute("BEGIN IMMEDIATE")
        do {
            for identity in upserts { try execute("INSERT OR IGNORE INTO identities VALUES(?)", identity) }
            for identity in absent { try execute("DELETE FROM identities WHERE identity=?", identity) }
            try execute("COMMIT")
        } catch { try? execute("ROLLBACK"); throw error }
    }
}
