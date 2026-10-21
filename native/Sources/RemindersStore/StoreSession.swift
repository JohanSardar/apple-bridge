import Foundation
import SourceSupport

public struct StoreReadResult: Sendable {
    public let snapshot: TypedStoreSnapshot
    public let timings: StoreTimings
    public let mode: String
    public let decodedRows: Int
}

public actor RemindersStoreSession {
    private let path: String
    private var connection: StoreConnection?
    private var fileIdentity = ""
    private var schemaVersion: Int64?
    private var dataVersion: Int64?
    private var historyToken: Int64?
    private var transactionToken: Int64?
    private var cached = TypedStoreSnapshot()
    private var versions: [StoreTable: [Int64: Int64]] = [:]
    private var initialized = false

    public init(path: String) throws {
        guard !path.isEmpty else { throw SourceFailure.invalid }
        self.path = path
    }

    private static func identity(_ path: String) throws -> String {
        let attributes = try FileManager.default.attributesOfItem(atPath: path)
        return "\(attributes[.systemNumber] ?? 0):\(attributes[.systemFileNumber] ?? 0)"
    }

    public func read(full: Bool = false) async throws -> StoreReadResult {
        let start = ProcessInfo.processInfo.systemUptime
        let identity = try Self.identity(path)
        if identity != fileIdentity || connection == nil {
            connection = try StoreConnection(path: path)
            fileIdentity = identity
            schemaVersion = nil
            initialized = false
        }
        guard let connection else { throw SourceFailure.unavailable }
        let opening = connection.timings.openMS
        connection.timings = StoreTimings()
        connection.timings.openMS = opening
        let version = try connection.scalar("PRAGMA data_version")
        try connection.execute("BEGIN")
        defer { try? connection.execute("ROLLBACK") }
        let schema = try connection.scalar("PRAGMA schema_version")
        let changedSchema = schema != schemaVersion
        if changedSchema { try connection.configure() }
        let hasHistory = Set(["Z_PK", "ZENTITY", "ZENTITYPK", "ZTRANSACTIONID"]).isSubset(of: connection.columns["ACHANGE"] ?? []) && (connection.columns["ATRANSACTION"] ?? []).contains("Z_PK")
        let latestHistory = hasHistory ? try connection.scalar("SELECT COALESCE(MAX(Z_PK),0) FROM ACHANGE") : 0
        let latestTransaction = hasHistory ? try connection.scalar("SELECT COALESCE(MAX(Z_PK),0) FROM ATRANSACTION") : 0
        var validToken = hasHistory && historyToken != nil && transactionToken != nil
        if let token = historyToken, token > 0, hasHistory {
            validToken = try connection.scalar("SELECT COUNT(*) FROM ACHANGE WHERE Z_PK=?", binding: token) == 1 && latestHistory >= token
        }
        if let token = transactionToken, token > 0, hasHistory {
            let anchorExists = try connection.scalar("SELECT COUNT(*) FROM ATRANSACTION WHERE Z_PK=?", binding: token) == 1
            validToken = validToken && anchorExists && latestTransaction >= token
        }
        var next = cached
        var nextVersions = versions
        var mode = "unchanged"
        var decoded = 0
        let needsFull = full || !initialized || changedSchema || (hasHistory && !validToken)
        if needsFull {
            mode = "full"
            next = TypedStoreSnapshot()
            for table in StoreTable.allCases { try connection.read(table, into: &next) }
            decoded = Self.count(next)
            nextVersions = hasHistory ? [:] : try readVersions()
        } else if version != dataVersion || latestHistory != historyToken {
            var changes: [StoreTable: Set<Int64>] = [:]
            if validToken, let token = historyToken, latestHistory > token {
                mode = "history"
                try connection.query("SELECT ZENTITY,ZENTITYPK FROM ACHANGE WHERE Z_PK>? ORDER BY Z_PK", binding: token) { row in
                    if let entity = row.integer(0), let key = row.integer(1), let table = connection.entityTables[entity] {
                        changes[table, default: []].insert(key)
                    }
                }
            }
            if changes.isEmpty {
                mode = "versions"
                let current = try readVersions()
                for table in StoreTable.allCases {
                    guard let now = current[table], let previous = versions[table] else {
                        Self.clear(table, from: &next)
                        try connection.read(table, into: &next)
                        decoded += Self.tableCount(table, next)
                        continue
                    }
                    for key in Set(now.keys).union(previous.keys) where now[key] != previous[key] { changes[table, default: []].insert(key) }
                }
                nextVersions = current
            }
            if mode != "full" {
                for (table, keys) in changes {
                    for key in keys {
                        Self.remove(table, key: key, from: &next)
                        try connection.read(table, key: key, into: &next)
                        decoded += 1
                    }
                }
                if mode == "history", !nextVersions.isEmpty {
                    for (table, keys) in changes {
                        for key in keys {
                            var value: Int64?
                            if (connection.columns[table.rawValue] ?? []).contains("Z_OPT") {
                                try connection.query("SELECT Z_OPT FROM " + table.rawValue + " WHERE Z_PK=?", binding: key) { value = $0.integer(0) }
                            }
                            nextVersions[table]?[key] = value
                        }
                    }
                }
            }
        }
        try connection.execute("COMMIT")
        cached = next
        versions = nextVersions
        initialized = true
        schemaVersion = schema
        dataVersion = version
        historyToken = hasHistory ? latestHistory : nil
        transactionToken = hasHistory ? latestTransaction : nil
        var timings = connection.timings
        connection.timings.openMS = 0
        timings.totalMS = (ProcessInfo.processInfo.systemUptime - start) * 1000
        timings.decodeMS = max(0, timings.totalMS - timings.queryMS - timings.openMS)
        return StoreReadResult(snapshot: next, timings: timings, mode: mode, decodedRows: decoded)
    }

    private func readVersions() throws -> [StoreTable: [Int64: Int64]] {
        guard let connection else { throw SourceFailure.unavailable }
        var result: [StoreTable: [Int64: Int64]] = [:]
        for table in StoreTable.allCases {
            guard (connection.columns[table.rawValue] ?? []).contains("Z_OPT") else { continue }
            var rows: [Int64: Int64] = [:]
            try connection.query("SELECT Z_PK,Z_OPT FROM " + table.rawValue) { row in
                if let key = row.integer(0), let version = row.integer(1) { rows[key] = version }
            }
            result[table] = rows
        }
        return result
    }

    private static func clear(_ table: StoreTable, from snapshot: inout TypedStoreSnapshot) {
        switch table {
        case .reminder: snapshot.reminders = [:]
        case .list: snapshot.lists = [:]
        case .section: snapshot.sections = [:]
        case .object: snapshot.objects = [:]
        case .label: snapshot.labels = [:]
        }
    }

    private static func tableCount(_ table: StoreTable, _ snapshot: TypedStoreSnapshot) -> Int {
        switch table {
        case .reminder: snapshot.reminders.count
        case .list: snapshot.lists.count
        case .section: snapshot.sections.count
        case .object: snapshot.objects.count
        case .label: snapshot.labels.count
        }
    }

    private static func count(_ snapshot: TypedStoreSnapshot) -> Int {
        snapshot.reminders.count + snapshot.lists.count + snapshot.sections.count + snapshot.objects.count + snapshot.labels.count
    }

    private static func remove(_ table: StoreTable, key: Int64, from snapshot: inout TypedStoreSnapshot) {
        switch table {
        case .reminder: snapshot.reminders.removeValue(forKey: key)
        case .list: snapshot.lists.removeValue(forKey: key)
        case .section: snapshot.sections.removeValue(forKey: key)
        case .object: snapshot.objects.removeValue(forKey: key)
        case .label: snapshot.labels.removeValue(forKey: key)
        }
    }
}
