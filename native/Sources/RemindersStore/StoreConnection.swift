import Foundation
import SQLite3
import SourceSupport

enum StoreTable: String, CaseIterable {
    case reminder = "ZREMCDREMINDER"
    case list = "ZREMCDBASELIST"
    case section = "ZREMCDBASESECTION"
    case object = "ZREMCDOBJECT"
    case label = "ZREMCDHASHTAGLABEL"

    var ancestor: String {
        switch self {
        case .reminder: "REMCDReminder"
        case .list: "REMCDBaseList"
        case .section: "REMCDBaseSection"
        case .object: "REMCDObject"
        case .label: "REMCDHashtagLabel"
        }
    }

    var fields: [String] {
        let common = ["Z_PK", "Z_ENT", "Z_OPT", "ZIDENTIFIER"]
        switch self {
        case .reminder: return common + ["ZDACALENDARITEMUNIQUEIDENTIFIER", "ZEXTERNALIDENTIFIER", "ZFLAGGED", "ZPARENTREMINDER", "ZLIST", "ZCREATIONDATE", "ZDUEDATEDELTAALERTSDATA"]
        case .list: return common + ["ZNAME", "ZBADGEEMBLEM", "ZISGROUP", "ZPARENTLIST", "ZPARENTLIST1", "ZDADISPLAYORDER", "ZISPINNEDBYCURRENTUSER", "ZPINNEDDATE", "ZSORTINGSTYLE", "ZSHOULDCATEGORIZEGROCERYITEMS", "ZGROCERYLOCALEID", "ZSHARINGSTATUS", "ZSHAREDOWNERNAME", "ZSMARTLISTTYPE", "ZFILTERDATA", "ZSECTIONIDSORDERINGASDATA", "ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA", "ZREMINDERIDSMERGEABLEORDERING", "ZREMINDERIDSMERGEABLEORDERING_V2_JSON"]
        case .section: return common + ["ZDISPLAYNAME", "ZLIST", "ZSMARTLIST"]
        case .object: return common + ["ZREMINDER", "ZREMINDER1", "ZREMINDER2", "ZREMINDER3", "ZREMINDER4", "ZREMINDER5", "ZHASHTAGLABEL", "ZLIST"]
        case .label: return ["Z_PK", "ZNAME"]
        }
    }
}

final class StoreConnection {
    private var database: OpaquePointer?
    private var statements: [String: OpaquePointer] = [:]
    private(set) var columns: [String: Set<String>] = [:]
    private(set) var entities: [Int64: String] = [:]
    private(set) var entityTables: [Int64: StoreTable] = [:]
    private var predicates: [StoreTable: String] = [:]
    var timings = StoreTimings()

    init(path: String) throws {
        let start = ProcessInfo.processInfo.systemUptime
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            if let database { sqlite3_close(database) }
            database = nil
            throw SourceFailure.permissionMissing
        }
        sqlite3_busy_timeout(database, 25)
        timings.openMS = (ProcessInfo.processInfo.systemUptime - start) * 1000
    }

    deinit {
        for statement in statements.values { sqlite3_finalize(statement) }
        sqlite3_close(database)
    }

    func execute(_ sql: String) throws {
        guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
    }

    func query(_ sql: String, binding: Int64? = nil, row: (StoreColumns) throws -> Void) throws {
        let statement: OpaquePointer
        if let cached = statements[sql] { statement = cached }
        else {
            var prepared: OpaquePointer?
            guard sqlite3_prepare_v2(database, sql, -1, &prepared, nil) == SQLITE_OK, let prepared else { throw SourceFailure.unsupported }
            statements[sql] = prepared
            statement = prepared
        }
        defer { sqlite3_reset(statement); sqlite3_clear_bindings(statement) }
        if let binding, sqlite3_bind_int64(statement, 1, binding) != SQLITE_OK { throw SourceFailure.unavailable }
        while true {
            let start = ProcessInfo.processInfo.systemUptime
            let status = sqlite3_step(statement)
            timings.queryMS += (ProcessInfo.processInfo.systemUptime - start) * 1000
            if status == SQLITE_DONE { return }
            guard status == SQLITE_ROW else { throw SourceFailure.corrupt }
            try row(StoreColumns(statement: statement))
        }
    }

    func scalar(_ sql: String, binding: Int64? = nil) throws -> Int64 {
        var value: Int64 = 0
        try query(sql, binding: binding) { value = $0.integer(0) ?? 0 }
        return value
    }

    func configure() throws {
        columns.removeAll()
        entities.removeAll()
        entityTables.removeAll()
        predicates.removeAll()
        var parents: [Int64: Int64] = [:]
        try query("SELECT Z_ENT,Z_NAME,Z_SUPER FROM Z_PRIMARYKEY") { row in
            guard let id = row.integer(0), let name = row.text(1), entities[id] == nil else { throw SourceFailure.corrupt }
            entities[id] = name
            parents[id] = row.integer(2)
        }
        for table in StoreTable.allCases {
            var schema: Set<String> = []
            try query("PRAGMA table_info(\(table.rawValue))") { if let name = $0.text(1) { schema.insert(name) } }
            guard schema.contains("Z_PK") else { throw SourceFailure.unsupported }
            columns[table.rawValue] = schema
            if table == .label {
                predicates[table] = "1"
                for (entity, name) in entities where name == table.ancestor { entityTables[entity] = table }
                continue
            }
            guard let root = entities.first(where: { $0.value == table.ancestor })?.key else { throw SourceFailure.unsupported }
            var descendants: Set<Int64> = [root]
            for _ in entities {
                let next = descendants.union(parents.compactMap { descendants.contains($0.value) ? $0.key : nil })
                if next == descendants { break }
                descendants = next
            }
            for entity in descendants { entityTables[entity] = table }
            let required = ["Z_ENT", "ZMARKEDFORDELETION", "ZCKNEEDSTOBEFETCHEDFROMCLOUD", "ZCKNEEDSINITIALFETCHFROMCLOUD"]
            guard Set(required).isSubset(of: schema) else { throw SourceFailure.unsupported }
            predicates[table] = "Z_ENT IN (" + descendants.sorted().map(String.init).joined(separator: ",") + ") AND COALESCE(ZMARKEDFORDELETION,0)=0 AND COALESCE(ZCKNEEDSTOBEFETCHEDFROMCLOUD,0)=0 AND COALESCE(ZCKNEEDSINITIALFETCHFROMCLOUD,0)=0"
        }
        for table in ["ATRANSACTION", "ACHANGE"] {
            var schema: Set<String> = []
            try query("PRAGMA table_info(\(table))") { if let name = $0.text(1) { schema.insert(name) } }
            columns[table] = schema
        }
    }

    func read(_ table: StoreTable, key: Int64? = nil, into snapshot: inout TypedStoreSnapshot) throws {
        let schema = columns[table.rawValue] ?? []
        let selected = table.fields.map { schema.contains($0) ? $0 : "NULL" }.joined(separator: ",")
        let sql = "SELECT " + selected + " FROM " + table.rawValue + " WHERE " + (predicates[table] ?? "0") + (key == nil ? " ORDER BY Z_PK" : " AND Z_PK=?")
        try query(sql, binding: key) { row in
            switch table {
            case .reminder:
                let id = row.identity
                snapshot.reminders[id.key] = StoreReminderRow(identity: id, calendarItemIdentifier: row.text(4), externalIdentifier: row.text(5), flagged: row.bool(6), parent: row.integer(7), list: row.integer(8), created: row.number(9), earlyReminder: row.data(10))
            case .list:
                let id = row.identity
                snapshot.lists[id.key] = StoreListRow(identity: id, name: row.text(4), emblem: row.text(5), isGroup: row.bool(6), parent: row.integer(7), alternateParent: row.integer(8), displayOrder: row.number(9), pinned: row.bool(10), pinnedDate: row.number(11), sortingStyle: row.scalar(12), groceries: row.bool(13), groceryLocale: row.text(14), sharingStatus: row.integer(15), sharedOwner: row.text(16), smartType: row.scalar(17), filter: row.data(18), sectionOrder: row.data(19), sectionMembership: row.data(20), ordering: row.data(21), orderingJSON: row.data(22))
            case .section:
                let id = row.identity
                snapshot.sections[id.key] = StoreSectionRow(identity: id, name: row.text(4), list: row.integer(5), smartList: row.integer(6))
            case .object:
                let id = row.identity
                snapshot.objects[id.key] = StoreObjectRow(identity: id, reminder: row.integer(4), reminder1: row.integer(5), reminder2: row.integer(6), reminder3: row.integer(7), reminder4: row.integer(8), reminder5: row.integer(9), hashtagLabel: row.integer(10), list: row.integer(11))
            case .label:
                let key = row.integer(0)!
                snapshot.labels[key] = StoreLabelRow(key: key, name: row.text(1))
            }
        }
    }
}
