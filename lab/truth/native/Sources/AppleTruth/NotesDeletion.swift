import Foundation
import SQLite3

extension AppleTruth {
    static func notesDeletionTargets(tag: String, folderKey: String?) throws -> [String] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if db != nil { sqlite3_close(db) }; throw NotesDeletionFailure.read
        }
        defer { sqlite3_close(db) }
        let sql: String, bindings: [String]
        if let folderKey {
            sql = "SELECT m.Z_UUID,'ICFolder',f.Z_PK FROM Z_METADATA m CROSS JOIN ZICCLOUDSYNCINGOBJECT f WHERE f.Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICFolder') AND f.ZIDENTIFIER=? AND COALESCE(f.ZMARKEDFORDELETION,0)=0"
            bindings = [folderKey]
        } else {
            sql = """
            SELECT m.Z_UUID,'ICNote',n.Z_PK FROM Z_METADATA m CROSS JOIN ZICCLOUDSYNCINGOBJECT n JOIN ZICCLOUDSYNCINGOBJECT f ON n.ZFOLDER=f.Z_PK
            WHERE n.Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICNote') AND COALESCE(n.ZMARKEDFORDELETION,0)=0
            AND substr(n.ZTITLE1,1,length(?))=? AND (f.ZTITLE2='Bridge Test' OR f.ZTITLE2=? OR substr(f.ZTITLE2,1,length(?))=?)
            """
            bindings = [tag,tag,"Bridge Test " + tag,"Bridge Test " + tag + " ","Bridge Test " + tag + " "]
        }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw NotesDeletionFailure.read }
        defer { sqlite3_finalize(statement) }
        for (index, value) in bindings.enumerated() {
            let result = value.withCString { sqlite3_bind_text(statement, Int32(index+1), $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard result == SQLITE_OK else { throw NotesDeletionFailure.read }
        }
        var ids: [String] = []
        while true {
            let result = sqlite3_step(statement)
            if result == SQLITE_DONE { break }
            guard result == SQLITE_ROW, let uuid = sqlite3_column_text(statement, 0), let entity = sqlite3_column_text(statement, 1) else { throw NotesDeletionFailure.read }
            ids.append("x-coredata://" + String(cString: uuid) + "/" + String(cString: entity) + "/p" + String(sqlite3_column_int64(statement, 2)))
        }
        return ids
    }

    static func notesDeletionVerified(_ ids: [String], path: String? = nil) throws -> Bool {
        let path = path ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if db != nil { sqlite3_close(db) }; throw NotesDeletionFailure.read
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 1000)
        func scalar(_ sql: String) throws -> String {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw NotesDeletionFailure.read }
            defer { sqlite3_finalize(statement) }
            guard sqlite3_step(statement) == SQLITE_ROW, let value = sqlite3_column_text(statement, 0) else { throw NotesDeletionFailure.read }
            return String(cString: value)
        }
        let uuid = try scalar("SELECT Z_UUID FROM Z_METADATA")
        for id in ids {
            let prefix = "x-coredata://" + uuid + "/"
            guard id.hasPrefix(prefix) else { throw NotesDeletionFailure.identity }
            let parts = id.dropFirst(prefix.count).split(separator: "/")
            guard parts.count == 2, ["ICNote", "ICFolder"].contains(String(parts[0])), parts[1].first == "p", let pk = Int(parts[1].dropFirst()), pk > 0 else { throw NotesDeletionFailure.identity }
            let start = parts[0] == "ICNote" ? "ZFOLDER" : "Z_PK"
            guard try scalar("SELECT count(*) FROM Z_PRIMARYKEY WHERE Z_NAME='\(parts[0])'") == "1" else { throw NotesDeletionFailure.read }
            let state = try scalar("""
            WITH RECURSIVE target AS (
              SELECT Z_PK,ZFOLDER,ZMARKEDFORDELETION FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=\(pk)
              AND Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='\(parts[0])')
            ), ancestors(pk,depth) AS (
              SELECT \(start),0 FROM target
              UNION ALL SELECT f.ZPARENT,a.depth+1 FROM ZICCLOUDSYNCINGOBJECT f JOIN ancestors a ON a.pk=f.Z_PK WHERE a.depth<128
            ) SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM target)
              OR EXISTS(SELECT 1 FROM target WHERE COALESCE(ZMARKEDFORDELETION,0)<>0)
              OR EXISTS(SELECT 1 FROM ancestors a JOIN ZICCLOUDSYNCINGOBJECT f ON a.pk=f.Z_PK WHERE f.ZFOLDERTYPE=1)
              THEN 'deleted' ELSE 'active' END
            """)
            if state != "deleted" { return false }
        }
        return true
    }
}
private enum NotesDeletionFailure: Error { case read, identity }
