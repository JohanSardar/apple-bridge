import Foundation
import RemindersStore

@MainActor
enum StoreSessionChecks {
    static func run(path: String, execute: (String) -> Void) async throws {
        execute("CREATE TABLE ATRANSACTION(Z_PK INTEGER PRIMARY KEY)")
        execute("CREATE TABLE ACHANGE(Z_PK INTEGER PRIMARY KEY,ZENTITY INTEGER,ZENTITYPK INTEGER,ZTRANSACTIONID INTEGER)")
        let reader = try RemindersStoreSession(path: path)
        let initial = try await reader.read()
        precondition(initial.mode == "full")
        precondition(initial.snapshot.reminders[10]?.identity.uuid == UUID(uuidString: "11111111-2222-3333-4444-555555555555"))
        precondition(initial.snapshot.reminders[10]?.flagged == false)
        precondition(initial.snapshot.lists[1]?.emblem == "star")
        let unchanged = try await reader.read()
        precondition(unchanged.mode == "unchanged")
        let steps: [(String, Int, Int)] = [
            ("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(15,101,'new','eventkit-new',0,1)", 101, 15),
            ("UPDATE ZREMCDREMINDER SET ZFLAGGED=1,Z_OPT=Z_OPT+1 WHERE Z_PK=15", 101, 15),
            ("UPDATE ZREMCDREMINDER SET ZPARENTREMINDER=10,Z_OPT=Z_OPT+1 WHERE Z_PK=15", 101, 15),
            ("UPDATE ZREMCDBASELIST SET ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA=X'01020304',Z_OPT=Z_OPT+1 WHERE Z_PK=1", 333, 1),
            ("DELETE FROM ZREMCDREMINDER WHERE Z_PK=15", 101, 15)
        ]
        for (index, step) in steps.enumerated() {
            let token = index + 1
            execute("BEGIN; " + step.0 + "; INSERT INTO ATRANSACTION VALUES(\(token)); INSERT INTO ACHANGE VALUES(\(token),\(step.1),\(step.2),\(token)); COMMIT")
            let incremental = try await reader.read()
            let full = try await RemindersStoreSession(path: path).read(full: true)
            precondition(incremental.mode == "history" && incremental.decodedRows == 1)
            precondition(incremental.snapshot == full.snapshot)
            let reference = try RemindersStore(path: path).read()
            precondition(reference.reminders.count == incremental.snapshot.reminders.count)
            for row in reference.reminders {
                let key = Int64(row["Z_PK"]!.number)
                precondition(incremental.snapshot.reminders[key]?.flagged == (row["ZFLAGGED"]?.number == 1))
                precondition(incremental.snapshot.reminders[key]?.parent == row["ZPARENTREMINDER"].flatMap { $0 == .null ? nil : Int64($0.number) })
            }
        }
        precondition(initial.snapshot.lists[1]?.sectionMembership == nil)
        let moved = try await reader.read()
        precondition(moved.snapshot.lists[1]?.sectionMembership == Data([1,2,3,4]))
        execute("DELETE FROM ACHANGE WHERE Z_PK<=5; DELETE FROM ATRANSACTION WHERE Z_PK<=5; INSERT INTO ATRANSACTION VALUES(6); INSERT INTO ACHANGE VALUES(6,101,10,6); UPDATE ZREMCDREMINDER SET ZFLAGGED=1,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let prunedPast = try await reader.read()
        let freshAfterPruning = try await RemindersStoreSession(path: path).read(full: true)
        precondition(prunedPast.mode == "full" && prunedPast.snapshot == freshAfterPruning.snapshot)
        execute("DELETE FROM ACHANGE; DELETE FROM ATRANSACTION; UPDATE ZREMCDREMINDER SET ZFLAGGED=1,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let truncated = try await reader.read()
        precondition(truncated.mode == "full" && truncated.snapshot.reminders[10]?.flagged == true)
        execute("DROP TABLE ACHANGE; DROP TABLE ATRANSACTION")
        let noHistory = try await reader.read()
        precondition(noHistory.mode == "full")
        execute("UPDATE ZREMCDREMINDER SET ZFLAGGED=0,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let fallback = try await reader.read()
        precondition(fallback.mode == "versions" && fallback.decodedRows == 2)
        let fallbackFull = try await RemindersStoreSession(path: path).read(full: true)
        precondition(fallback.snapshot == fallbackFull.snapshot)
        execute("UPDATE ZREMCDREMINDER SET ZMARKEDFORDELETION=1,Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        let deleted = try await reader.read()
        precondition(deleted.snapshot.reminders[11] == nil)
        execute("ALTER TABLE ZREMCDREMINDER ADD COLUMN ZUNUSED TEXT")
        let schemaChanged = try await reader.read()
        precondition(schemaChanged.mode == "full")
        let scope = RemindersCaptureScope(paths: { [path] in [path] })
        let initialScope = await scope.read()
        precondition(initialScope == nil)
        let emptyScope = await scope.read()
        precondition(emptyScope == [])
        execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(16,101,X'22222222333344445555666666666666','22222222-3333-4444-5555-666666666666',0,1)")
        let createdScope = await scope.read()
        precondition(createdScope == ["list"])
        execute("INSERT INTO ZREMCDOBJECT(Z_PK,Z_ENT,ZREMINDER3) VALUES(31,934,16)")
        let childScope = await scope.read()
        precondition(childScope == ["list"])
        execute("UPDATE ZREMCDOBJECT SET ZREMINDER3=NULL,ZLIST=1,Z_OPT=Z_OPT+1 WHERE Z_PK=31")
        let childListScope = await scope.read()
        precondition(childListScope == ["list"])
        execute("DELETE FROM ZREMCDOBJECT WHERE Z_PK=31")
        let childDeletedScope = await scope.read()
        precondition(childDeletedScope == ["list"])
        execute("UPDATE ZREMCDOBJECT SET Z_OPT=Z_OPT+1 WHERE Z_PK=30")
        let relatedScope = await scope.read()
        precondition(relatedScope == nil)
        execute("UPDATE ZREMCDREMINDER SET ZLIST=2,Z_OPT=Z_OPT+1 WHERE Z_PK=16")
        let movedScope = await scope.read()
        precondition(movedScope == ["list", "smart"])
        execute("DELETE FROM ZREMCDREMINDER WHERE Z_PK=16")
        let deletedScope = await scope.read()
        precondition(deletedScope == ["smart"])
        execute("ALTER TABLE ZREMCDREMINDER ADD COLUMN ZSCOPEUNUSED TEXT")
        let fallbackScope = await scope.read()
        precondition(fallbackScope == nil)
        print("capture_scope_create_move_delete_unchanged_schema_fallback: PASS")
        print("typed_full_equals_history_create_flag_nest_section_move_delete: PASS")
        print("history_pruned_past_token_label_without_opt_version_fallback_schema_change: PASS")
    }
}
