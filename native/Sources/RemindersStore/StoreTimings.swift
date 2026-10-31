import Foundation
import SourceSupport

public struct StoreTimings: Sendable {
    public var openMS = 0.0
    public var backupMS = 0.0
    public var queryMS = 0.0
    public var decodeMS = 0.0
    public var closeMS = 0.0
    public var totalMS = 0.0

    public mutating func add(_ other: StoreTimings) {
        openMS += other.openMS
        backupMS += other.backupMS
        queryMS += other.queryMS
        decodeMS += other.decodeMS
        closeMS += other.closeMS
        totalMS += other.totalMS
    }

    public var json: JSON {
        .object(["connection_open_ms": .number(openMS), "backup_ms": .number(backupMS), "query_ms": .number(queryMS), "decode_ms": .number(decodeMS), "close_ms": .number(closeMS), "total_ms": .number(totalMS)])
    }
}

#if LAB
public enum RemindersStoreProfile {
    public static func run(iterations: Int = 200) throws -> JSON {
        guard iterations == 200 else { throw SourceFailure.invalid }
        let paths = try RemindersStore.paths()
        guard !paths.isEmpty else { throw SourceFailure.unavailable }
        func sample() throws -> StoreTimings {
            try autoreleasepool {
                var total = StoreTimings()
                for path in paths { total.add(try RemindersStore(path: path).profile().timings) }
                return total
            }
        }
        let start = ProcessInfo.processInfo.systemUptime
        let cold = try sample()
        var warm: [StoreTimings] = []
        for _ in 0..<iterations { warm.append(try sample()) }
        func summarize(_ values: [Double]) -> JSON {
            let sorted = values.sorted()
            return .object(["samples": .number(Double(sorted.count)), "p50_ms": .number(sorted[99]), "p95_ms": .number(sorted[189]), "max_ms": .number(sorted.last ?? 0)])
        }
        return .object([
            "mode": .string("backup_per_read_baseline"), "cold": cold.json,
            "warm": .object([
                "connection_open": summarize(warm.map(\.openMS)), "backup": summarize(warm.map(\.backupMS)),
                "query": summarize(warm.map(\.queryMS)), "decode": summarize(warm.map(\.decodeMS)),
                "close": summarize(warm.map(\.closeMS)), "total": summarize(warm.map(\.totalMS))
            ]),
            "iterations": .number(Double(iterations)),
            "profile_total_ms": .number((ProcessInfo.processInfo.systemUptime - start) * 1000)
        ])
    }
}


extension RemindersStoreProfile {
    @MainActor
    public static func typed(iterations: Int = 200) async throws -> JSON {
        guard iterations == 200 else { throw SourceFailure.invalid }
        let start = ProcessInfo.processInfo.systemUptime
        let paths = try RemindersStore.paths()
        guard !paths.isEmpty else { throw SourceFailure.unavailable }
        let sessions = try paths.map { try RemindersStoreSession(path: $0) }
        func sample(full: Bool) async throws -> StoreTimings {
            var total = StoreTimings()
            for session in sessions { total.add(try await session.read(full: full).timings) }
            return total
        }
        let cold = try await sample(full: true)
        var full: [StoreTimings] = []
        var incremental: [StoreTimings] = []
        for _ in 0..<iterations { full.append(try await sample(full: true)) }
        for _ in 0..<iterations { incremental.append(try await sample(full: false)) }
        func summarize(_ values: [Double]) -> JSON {
            let sorted = values.sorted()
            return .object(["samples": .number(Double(sorted.count)), "p50_ms": .number(sorted[99]), "p95_ms": .number(sorted[189]), "max_ms": .number(sorted.last ?? 0)])
        }
        func timings(_ values: [StoreTimings]) -> JSON {
            .object(["connection_open": summarize(values.map(\.openMS)), "backup": summarize(values.map(\.backupMS)), "query": summarize(values.map(\.queryMS)), "decode": summarize(values.map(\.decodeMS)), "total": summarize(values.map(\.totalMS))])
        }
        var result = try await changedCopy(paths: paths).object
        for (key, value) in ["mode": JSON.string("persistent_typed"), "cold": cold.json, "warm_full": timings(full), "warm_incremental_unchanged": timings(incremental), "iterations": .number(Double(iterations)), "profile_total_ms": .number((ProcessInfo.processInfo.systemUptime - start) * 1000)] { result[key] = value }
        return .object(result)
    }
}
#endif
