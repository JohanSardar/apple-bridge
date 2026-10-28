import Foundation
import SQLite3
import SourceSupport

@main
struct Checks {
    static func require(_ value: Bool) throws {
        guard value else { throw SourceFailure.invalid }
    }
    static func rejects(_ action: () throws -> Void) throws {
        do { try action() } catch { return }
        throw SourceFailure.invalid
    }
    static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let path = root.appendingPathComponent("store.sqlite").path
        var db: OpaquePointer?
        try require(sqlite3_open(path, &db) == SQLITE_OK)
        defer { sqlite3_close(db) }
        func execute(_ sql: String) throws { try require(sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK) }
        try execute("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE metadata(id TEXT); INSERT INTO metadata VALUES('first'); CREATE TABLE items(id TEXT); INSERT INTO items VALUES('a');")
        let identitySQL = "SELECT id FROM metadata"
        let reader = try SQLiteSnapshot(path: path, generationSQL: identitySQL)
        try reader.requireSchema(["items": ["id"]])
        try rejects { try reader.requireSchema(["items": ["missing"]]) }
        try rejects { try reader.requireSchema(["absent": []]) }
        try rejects { _ = try reader.query("DELETE FROM items") }
        try rejects { _ = try reader.query("SELECT missing FROM items") }
        let scalars = try reader.query("SELECT 9223372036854775807 AS big, NULL AS empty, x'' AS bytes, ? AS text", values: ["a\0b"])
        try require(scalars.rows[0]["big"] == .integer(Int64.max))
        try require(scalars.rows[0]["empty"] == .null && scalars.rows[0]["bytes"] == .blob(Data()))
        try require(scalars.rows[0]["text"] == .text("a\0b"))
        let baseline = reader.generation
        let full = try CompleteScan.read(reader: reader, sql: "SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "items", previousGeneration: baseline, previousKeys: ["a", "b"], maxRows: 10, timeout: 2)
        try require(full.absentKeys == ["b"])
        let partial = try CompleteScan.read(reader: reader, sql: "SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "items", previousGeneration: baseline, previousKeys: ["a", "b"], maxRows: 0, timeout: 2)
        try require(!partial.complete && partial.absentKeys.isEmpty)
        let expired = try CompleteScan.read(reader: reader, sql: "SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "items", previousGeneration: baseline, previousKeys: ["b"], maxRows: 10, timeout: 0)
        try require(!expired.complete && expired.absentKeys.isEmpty)
        let scoped = try CompleteScan.read(reader: reader, sql: "SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "other", previousGeneration: baseline, previousKeys: ["b"], maxRows: 10, timeout: 2)
        try require(scoped.absentKeys.isEmpty)
        try rejects {
            _ = try CompleteScan.read(reader: reader, sql: "SELECT id FROM items UNION ALL SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "items", previousGeneration: baseline, previousKeys: ["b"], maxRows: 10, timeout: 2)
        }
        let watcher = DatabaseWatcher(path: path, coalescing: .milliseconds(20), pollInterval: .seconds(30))
        defer { watcher.close() }
        let observed = Task {
            for await _ in watcher.changes { return true }
            return false
        }
        try execute("INSERT INTO items VALUES('c')")
        let timeout = Task { try await Task.sleep(for: .seconds(3)); watcher.close() }
        try require(await observed.value)
        timeout.cancel()
        let fresh = try SQLiteSnapshot(path: path, generationSQL: identitySQL)
        try require(fresh.generation == baseline)
        try require(try fresh.query("SELECT id FROM items").rows.count == 2)
        try require(try reader.query("SELECT id FROM items").rows.count == 1)
        try execute("UPDATE metadata SET id='rebuilt'; DELETE FROM items")
        try require(try !reader.isCurrent())
        let rebuilt = try SQLiteSnapshot(path: path, generationSQL: identitySQL)
        try require(rebuilt.generation != baseline)
        let changed = try CompleteScan.read(reader: rebuilt, sql: "SELECT id FROM items", keyColumn: "id", scope: "items", previousScope: "items", previousGeneration: baseline, previousKeys: ["a"], maxRows: 10, timeout: 2)
        try require(changed.absentKeys.isEmpty)
        try execute("ALTER TABLE items ADD COLUMN value TEXT")
        let schemaChanged = try SQLiteSnapshot(path: path, generationSQL: identitySQL)
        try require(schemaChanged.generation != rebuilt.generation)
        let expensive = try schemaChanged.query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n", timeout: 0.005)
        try require(!expensive.complete)
        let otherPath = root.appendingPathComponent("replacement.sqlite").path
        var other: OpaquePointer?
        try require(sqlite3_open(otherPath, &other) == SQLITE_OK)
        try require(sqlite3_exec(other, "CREATE TABLE metadata(id TEXT); INSERT INTO metadata VALUES('rebuilt'); CREATE TABLE items(id TEXT)", nil, nil, nil) == SQLITE_OK)
        sqlite3_close(other)
        try FileManager.default.removeItem(atPath: path)
        try FileManager.default.moveItem(atPath: otherPath, toPath: path)
        try require(try !rebuilt.isCurrent())
        let writer = try BlobWriter(directory: root.appendingPathComponent("blobs"))
        let blob = try writer.stage(Data("abc".utf8))
        try require(blob.digest == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        let staged = try Data(contentsOf: blob.file)
        try require(blob.size == 3 && staged == Data("abc".utf8))
        let copy = try writer.stage(blob.file)
        try require(copy.digest == blob.digest)
        let large = Data(repeating: 42, count: 400_000)
        let largeBlob = try writer.stage(large)
        let largeCopy = try writer.stage(largeBlob.file)
        try require(largeCopy.digest == largeBlob.digest && largeCopy.size == Int64(large.count))
        let link = root.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: blob.file)
        try rejects { _ = try writer.stage(link) }
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try writer.stage(Data("cancelled".utf8))
        }
        do { _ = try await cancelled.value; throw SourceFailure.invalid }
        catch is CancellationError {}
        print("all SourceSupport checks passed")
    }
}
