import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StorePerformanceChecks {
    static func run(path: String, execute: (String) -> Void, report: String) async throws {
        execute("CREATE TABLE ATRANSACTION(Z_PK INTEGER PRIMARY KEY)")
        execute("CREATE TABLE ACHANGE(Z_PK INTEGER PRIMARY KEY,ZENTITY INTEGER,ZENTITYPK INTEGER,ZTRANSACTIONID INTEGER)")
        execute("BEGIN")
        for key in 1000..<2000 {
            execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(\(key),101,'fixture-\(key)','eventkit-fixture-\(key)',0,1)")
        }
        execute("COMMIT")
        let reader = try RemindersStoreSession(path: path)
        let cold = try await reader.read()
        var full: [StoreTimings] = []
        var unchanged: [StoreTimings] = []
        for _ in 0..<200 { full.append(try await reader.read(full: true).timings) }
        for _ in 0..<200 {
            let result = try await reader.read()
            precondition(result.mode == "unchanged" && result.decodedRows == 0)
            unchanged.append(result.timings)
        }
        var batches: [Int: [StoreTimings]] = [:]
        var token = 0
        var changeKey = 1
        for size in [1, 10, 100, 1000] {
            for _ in 0..<200 {
                token += 1
                execute("BEGIN; UPDATE ZREMCDREMINDER SET ZFLAGGED=\(token % 2),Z_OPT=Z_OPT+1 WHERE Z_PK>=1000 AND Z_PK<\(1000 + size); INSERT INTO ATRANSACTION VALUES(\(token)); INSERT INTO ACHANGE SELECT Z_PK-1000+\(changeKey),101,Z_PK,\(token) FROM ZREMCDREMINDER WHERE Z_PK>=1000 AND Z_PK<\(1000 + size); COMMIT")
                changeKey += size
                let result = try await reader.read()
                precondition(result.mode == "history" && result.decodedRows == size)
                precondition(result.snapshot.reminders[1000]?.flagged == (token % 2 == 1))
                batches[size, default: []].append(result.timings)
            }
            let incremental = try await reader.read()
            let reference = try await RemindersStoreSession(path: path).read(full: true)
            precondition(incremental.snapshot == reference.snapshot)
        }
        func summarize(_ values: [Double]) -> JSON {
            let sorted = values.sorted()
            return .object(["samples": .integer(200), "p50_ms": .number(sorted[99]), "p95_ms": .number(sorted[189]), "max_ms": .number(sorted.last!)])
        }
        func timings(_ values: [StoreTimings]) -> JSON {
            .object(["query": summarize(values.map(\.queryMS)), "decode": summarize(values.map(\.decodeMS)), "total": summarize(values.map(\.totalMS))])
        }
        var reportFields: [String: JSON] = ["mode": .string("synthetic_1000_typed"), "cold": cold.timings.json, "warm_full": timings(full), "warm_incremental_unchanged": timings(unchanged), "iterations": .integer(200), "change_sized_history_proven": .bool(true)]
        for size in [1, 10, 100, 1000] { reportFields["warm_incremental_changed_\(size)"] = timings(batches[size]!) }
        let result = JSON.object(reportFields)
        let beforeCopy = try await RemindersStoreSession(path: path).read(full: true)
        let copied = try await RemindersStoreProfile.changedCopy(paths: [path, path])
        precondition(copied["private_native_copy"].bool && copied["change_sized_history_proven"].bool && copied["source_stores"].number == 2)
        for size in [1, 10, 100, 1000] { precondition(copied["warm_incremental_changed_\(size)"]["total"]["samples"].number == 200) }
        let afterCopy = try await RemindersStoreSession(path: path).read(full: true)
        precondition(beforeCopy.snapshot == afterCopy.snapshot)
        try JSONEncoder().encode(result).write(to: URL(fileURLWithPath: report), options: .atomic)
    }
}
