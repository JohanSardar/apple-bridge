import Foundation
import SQLite3
import RemindersStore
import SourceSupport

@main
struct RemindersStoreChecks {
    @MainActor
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("bridge-reminders-store-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("fixture.sqlite").path
        let fixture = SyntheticReminderDatabase(path: path)
        fixture.execute("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0")
        fixture.execute("CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER PRIMARY KEY,Z_NAME TEXT,Z_SUPER INTEGER)")
        let entities = [(101,"REMCDReminder",0),(207,"REMCDBaseList",0),(333,"REMCDList",207),(408,"REMCDSmartList",207),(501,"REMCDBaseSection",0),(605,"REMCDListSection",501),(710,"REMCDObject",0),(811,"REMCDHashtag",710),(934,"REMCDImageAttachment",710)]
        for (id, name, parent) in entities {
            fixture.execute("INSERT INTO Z_PRIMARYKEY VALUES (\(id),'\(name)',\(parent))")
        }
        let common = "Z_PK INTEGER PRIMARY KEY,Z_OPT INTEGER DEFAULT 1,Z_ENT INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZCKNEEDSTOBEFETCHEDFROMCLOUD INTEGER DEFAULT 0,ZCKNEEDSINITIALFETCHFROMCLOUD INTEGER DEFAULT 0"
        fixture.execute("CREATE TABLE ZREMCDREMINDER(" + common + ",ZIDENTIFIER BLOB,ZDACALENDARITEMUNIQUEIDENTIFIER TEXT,ZEXTERNALIDENTIFIER TEXT,ZFLAGGED INTEGER,ZPARENTREMINDER INTEGER,ZLIST INTEGER,ZICSDISPLAYORDER INTEGER,ZCREATIONDATE REAL)")
        fixture.execute("CREATE TABLE ZREMCDBASELIST(" + common + ",ZIDENTIFIER BLOB,ZNAME TEXT,ZBADGEEMBLEM TEXT,ZCOLOR BLOB,ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA BLOB,ZISGROUP INTEGER,ZPARENTLIST INTEGER,ZDADISPLAYORDER INTEGER,ZISPINNEDBYCURRENTUSER INTEGER,ZSORTINGSTYLE TEXT,ZPINNEDDATE REAL,ZSHOULDCATEGORIZEGROCERYITEMS INTEGER,ZGROCERYLOCALEID TEXT,ZSHARINGSTATUS INTEGER,ZSHAREDOWNERNAME TEXT)")
        fixture.execute("CREATE TABLE ZREMCDBASESECTION(" + common + ",ZIDENTIFIER BLOB,ZDISPLAYNAME TEXT,ZLIST INTEGER)")
        fixture.execute("CREATE TABLE ZREMCDOBJECT(" + common + ",ZIDENTIFIER BLOB,ZREMINDER3 INTEGER,ZHASHTAGLABEL INTEGER)")
        fixture.execute("CREATE TABLE ZREMCDHASHTAGLABEL(Z_PK INTEGER PRIMARY KEY,ZNAME TEXT)")
        fixture.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        fixture.execute("INSERT INTO ZREMCDBASELIST(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME,ZBADGEEMBLEM,ZISGROUP,ZDADISPLAYORDER) VALUES(1,333,'list','Bridge Test','star',0,12),(2,408,'smart','Bridge Test Smart','flag',0,14)")
        fixture.execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZEXTERNALIDENTIFIER,ZFLAGGED,ZPARENTREMINDER,ZLIST,ZICSDISPLAYORDER,ZCREATIONDATE) VALUES(10,101,X'11111111222233334444555555555555','eventkit-parent','external-parent',1,NULL,1,3,800000000.5),(11,101,'child','eventkit-child','external-child',0,10,1,4,800000001)")
        fixture.execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZMARKEDFORDELETION) VALUES(12,101,1)")
        fixture.execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZCKNEEDSTOBEFETCHEDFROMCLOUD) VALUES(13,101,1)")
        fixture.execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZCKNEEDSINITIALFETCHFROMCLOUD) VALUES(14,101,1)")
        fixture.execute("INSERT INTO ZREMCDBASESECTION(Z_PK,Z_ENT,ZIDENTIFIER,ZDISPLAYNAME,ZLIST) VALUES(20,605,'section','Bridge Test Section',1)")
        fixture.execute("INSERT INTO ZREMCDOBJECT(Z_PK,Z_ENT,ZREMINDER3,ZHASHTAGLABEL) VALUES(30,811,11,40)")
        fixture.execute("INSERT INTO ZREMCDHASHTAGLABEL(Z_PK,ZNAME) VALUES(40,'bridge-test')")
        fixture.execute("UPDATE ZREMCDBASELIST SET ZISPINNEDBYCURRENTUSER=1,ZPINNEDDATE=800000002,ZSORTINGSTYLE='manual',ZSHOULDCATEGORIZEGROCERYITEMS=1,ZGROCERYLOCALEID='en_US',ZSHARINGSTATUS=2,ZSHAREDOWNERNAME='Bridge Test Owner',ZCOLOR=X'0102' WHERE Z_PK=1")
        let reader = RemindersStore(path: path)
        let snapshot = try reader.read()
        precondition(snapshot.reminders.count == 2 && snapshot.lists.count == 2 && snapshot.sections.count == 1)
        precondition(snapshot.entities[408] == "REMCDSmartList" && snapshot.entities[811] == "REMCDHashtag")
        precondition(snapshot.reminders[0]["ZIDENTIFIER"] == .string("11111111-2222-3333-4444-555555555555"))
        precondition(snapshot.reminders[0]["ZFLAGGED"] == .integer(1))
        precondition(snapshot.reminders[1]["ZPARENTREMINDER"] == .integer(10))
        precondition(snapshot.reminders[0]["ZDACALENDARITEMUNIQUEIDENTIFIER"] == .string("eventkit-parent"))
        precondition(snapshot.reminders[0]["ZEXTERNALIDENTIFIER"] == .string("external-parent"))
        precondition(snapshot.reminders[0]["ZCREATIONDATE"] == .number(800000000.5))
        precondition(snapshot.lists[0]["ZBADGEEMBLEM"] == .string("star"))
        precondition(snapshot.objects[0]["ZREMINDER3"] == .integer(11) && snapshot.labels[0]["ZNAME"] == .string("bridge-test"))
        precondition(snapshot.lists[0]["ZCOLOR"] == .string("AQI="))
        precondition(snapshot.reminders[0]["ZICSDISPLAYORDER"] == .integer(3))
        let parentKeys = [10: "eventkit-parent", 11: "eventkit-child"]
        let parentFields = snapshot.reminderFields(snapshot.reminders[0], parentKeys: parentKeys)
        let childFields = snapshot.reminderFields(snapshot.reminders[1], parentKeys: parentKeys)
        precondition(parentFields["flagged"] == .bool(true) && parentFields["parent_id"] == .null)
        precondition(childFields["parent_id"]["$ref"]["native_key"] == .string("eventkit-parent"))
        let listFields = snapshot.listFields(snapshot.lists[0], listKeys: [1: "list", 2: "smart"])
        precondition(listFields["badge_emblem"] == .string("star") && listFields["display_order"] == .integer(12))
        precondition(listFields["pinned"] == .bool(true) && listFields["sorting_style"] == .string("manual"))
        precondition(listFields["categorize_groceries"] == .bool(true) && listFields["grocery_locale"] == .string("en_US"))
        precondition(listFields["sharing_status"] == .integer(2) && listFields["shared_owner_name"] == .string("Bridge Test Owner"))
        let contractChecks = [
            ("unsupported_store_color_is_null", listFields["store_color"] == .null),
            ("ics_order_hint_stays_internal", parentFields.object["ics_display_order"] == nil)
        ]
        for (name, passed) in contractChecks { print("\(name): \(passed ? "PASS" : "FAIL")") }
        if contractChecks.contains(where: { !$0.1 }) { exit(1) }
        if let index = CommandLine.arguments.firstIndex(of: "--report"), index + 1 < CommandLine.arguments.count {
            var parent = parentFields.object
            parent["title"] = .string("Bridge Test Parent")
            parent["completed"] = .bool(false)
            parent["list_id"] = RemindersStoreSnapshot.reference("reminders.list", "list")
            var child = childFields.object
            child["title"] = .string("Bridge Test Child")
            child["completed"] = .bool(false)
            child["list_id"] = RemindersStoreSnapshot.reference("reminders.list", "list")
            var list = listFields.object
            list["title"] = .string("Bridge Test List")
            list["writable"] = .bool(true)
            func observation(_ collection: String, _ key: String, _ data: JSON) -> JSON {
                .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "data": data])
            }
            let result = JSON.object(["observations": .array([
                observation("reminders.list", "list", .object(list)),
                observation("reminders.reminder", "eventkit-parent", .object(parent)),
                observation("reminders.reminder", "eventkit-child", .object(child))
            ])])
            try JSONEncoder().encode(result).write(to: URL(fileURLWithPath: CommandLine.arguments[index + 1]), options: .atomic)
        }
        fixture.execute("UPDATE ZREMCDREMINDER SET ZFLAGGED=0 WHERE Z_PK=10")
        precondition(snapshot.reminders[0]["ZFLAGGED"] == .integer(1))
        let refreshed = try reader.read()
        precondition(refreshed.reminders[0]["ZFLAGGED"] == .integer(0))
        try await StoreSessionChecks.run(path: path, execute: fixture.execute)
        if let index = CommandLine.arguments.firstIndex(of: "--perf-report"), index + 1 < CommandLine.arguments.count {
            try await StorePerformanceChecks.run(path: path, execute: fixture.execute, report: CommandLine.arguments[index + 1])
        }
        print("RemindersStore: synthetic schema, arbitrary entity IDs, WAL backup, isolation, deleted and pending exclusion passed")
    }
}

private final class SyntheticReminderDatabase {
    private var database: OpaquePointer?

    init(path: String) {
        precondition(sqlite3_open(path, &database) == SQLITE_OK)
    }

    deinit { sqlite3_close(database) }

    func execute(_ sql: String) {
        precondition(sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK)
    }
}
