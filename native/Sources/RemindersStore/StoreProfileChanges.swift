import Foundation
import SQLite3
import SourceSupport

#if LAB
private final class StoreProfileCopy {
    private var database: OpaquePointer?
    init(source: String, destination: String) throws {
        var live: OpaquePointer?
        guard sqlite3_open_v2(source, &live, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            if let live { sqlite3_close(live) }
            throw SourceFailure.permissionMissing
        }
        defer { sqlite3_close(live) }
        sqlite3_busy_timeout(live, 250)
        var completed = false
        defer { if !completed { sqlite3_close(database); database = nil } }
        guard sqlite3_open(destination, &database) == SQLITE_OK else { throw SourceFailure.unavailable }
        guard let backup = sqlite3_backup_init(database, "main", live, "main") else { throw SourceFailure.unavailable }
        let step = sqlite3_backup_step(backup, -1)
        let finish = sqlite3_backup_finish(backup)
        guard step == SQLITE_DONE && finish == SQLITE_OK else { throw SourceFailure.unavailable }
        try execute("PRAGMA journal_mode=WAL")
        completed = true
    }
    deinit { sqlite3_close(database) }
    func execute(_ sql: String) throws {
        guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
    }
    func scalar(_ sql: String) throws -> Int64 {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unsupported }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW else { throw SourceFailure.corrupt }
        return sqlite3_column_int64(statement, 0)
    }
}

extension RemindersStoreProfile {
    public static func changedCopy(paths: [String]) async throws -> JSON {
        guard !paths.isEmpty else { throw SourceFailure.invalid }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-reminders-profile-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        var stores: [(writer: StoreProfileCopy, reader: RemindersStoreSession, original: StoreReadResult, path: String)] = []
        for (index, path) in paths.enumerated() {
            let copiedPath = directory.appendingPathComponent("store-\(index).sqlite").path
            let writer = try StoreProfileCopy(source: path, destination: copiedPath)
            let reader = try RemindersStoreSession(path: copiedPath)
            stores.append((writer, reader, try await reader.read(full: true), copiedPath))
        }
        let target = stores.indices.max(by: { stores[$0].original.snapshot.reminders.count < stores[$1].original.snapshot.reminders.count })!
        let (writer, reader, original, _) = stores[target]
        guard let entity = original.entities.first(where: { $0.value == "REMCDReminder" })?.key,
              Set(["Z_PK", "Z_ENT", "Z_OPT", "ZIDENTIFIER", "ZFLAGGED"]).isSubset(of: original.columns["ZREMCDREMINDER"] ?? []),
              Set(["Z_PK", "ZENTITY", "ZENTITYPK", "ZTRANSACTIONID"]).isSubset(of: original.columns["ACHANGE"] ?? []),
              original.columns["ATRANSACTION"]?.contains("Z_PK") == true else { throw SourceFailure.unsupported }
        let firstKey = try writer.scalar("SELECT COALESCE(MAX(Z_PK),0)+1 FROM ZREMCDREMINDER")
        try writer.execute("BEGIN")
        for offset in 0..<1000 {
            try writer.execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,Z_OPT,ZIDENTIFIER,ZFLAGGED) VALUES(\(firstKey + Int64(offset)),\(entity),1,'[ab-test] \(UUID().uuidString)',0)")
        }
        try writer.execute("COMMIT")
        let seeded = try await reader.read(full: true)
        guard seeded.snapshot.reminders.count == original.snapshot.reminders.count + 1000 else { throw SourceFailure.corrupt }
        var transaction = try writer.scalar("SELECT COALESCE(MAX(Z_PK),0) FROM ATRANSACTION")
        var change = try writer.scalar("SELECT COALESCE(MAX(Z_PK),0)+1 FROM ACHANGE")
        var fields: [String: JSON] = ["private_native_copy": .bool(true), "fixture_reminders": .integer(1000), "source_stores": .integer(UInt64(stores.count)), "source_reminders": .integer(UInt64(stores.reduce(0) { $0 + $1.original.snapshot.reminders.count }))]
        for size in [1, 10, 100, 1000] {
            var samples: [Double] = []
            for _ in 0..<200 {
                try Task.checkCancellation()
                transaction += 1
                try writer.execute("BEGIN; UPDATE ZREMCDREMINDER SET ZFLAGGED=\(transaction % 2),Z_OPT=COALESCE(Z_OPT,0)+1 WHERE Z_PK>=\(firstKey) AND Z_PK<\(firstKey + Int64(size)); INSERT INTO ATRANSACTION(Z_PK) VALUES(\(transaction)); INSERT INTO ACHANGE(Z_PK,ZENTITY,ZENTITYPK,ZTRANSACTIONID) SELECT Z_PK-\(firstKey)+\(change),\(entity),Z_PK,\(transaction) FROM ZREMCDREMINDER WHERE Z_PK>=\(firstKey) AND Z_PK<\(firstKey + Int64(size)); COMMIT")
                change += Int64(size)
                var elapsed = 0.0
                for (index, store) in stores.enumerated() {
                    let result = try await store.reader.read()
                    if index == target {
                        guard result.mode == "history", result.decodedRows == size, result.snapshot.reminders[firstKey]?.flagged == (transaction % 2 == 1) else { throw SourceFailure.corrupt }
                    } else if result.mode != "unchanged" || result.decodedRows != 0 { throw SourceFailure.corrupt }
                    elapsed += result.timings.totalMS
                }
                samples.append(elapsed)
            }
            for store in stores {
                let incremental = try await store.reader.read()
                let reference = try await RemindersStoreSession(path: store.path).read(full: true)
                guard incremental.snapshot == reference.snapshot else { throw SourceFailure.corrupt }
            }
            samples.sort()
            fields["warm_incremental_changed_\(size)"] = .object(["total": .object(["samples": .integer(200), "p50_ms": .number(samples[99]), "p95_ms": .number(samples[189]), "max_ms": .number(samples[199])])])
        }
        fields["change_sized_history_proven"] = .bool(true)
        return .object(fields)
    }
}
#endif
