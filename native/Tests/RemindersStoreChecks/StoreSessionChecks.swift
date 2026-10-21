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
        print("typed_full_equals_history_create_flag_nest_section_move_delete: PASS")
        print("history_pruned_past_token_label_without_opt_version_fallback_schema_change: PASS")
    }
}
