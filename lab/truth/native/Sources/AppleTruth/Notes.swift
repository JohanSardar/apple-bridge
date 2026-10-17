import Foundation
import Darwin
import SQLite3

extension AppleTruth {
    @MainActor
    static func notes(_ verb: String, _ request: [String: Any]) throws -> [String: Any] {
        func quoted(_ text: String) -> String { "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\"" }
        func run(_ text: String) throws -> NSAppleEventDescriptor {
            var error: NSDictionary?
            guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application \"Notes\"\n" + text + "\nend tell\nend timeout") else { throw NotesTruthFailure.invalid }
            let value = script.executeAndReturnError(&error)
            if let error { throw NSError(domain: "NotesAppleScript", code: error["NSAppleScriptErrorNumber"] as? Int ?? 0) }
            return value
        }
        let tag = request["tag"] as? String ?? ""
        let action = request["action"] as? String ?? ""
        let title = request["title"] as? String ?? ""
        if verb == "fixture" {
            guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\n"), tag.count > 11 else { throw NotesTruthFailure.invalid }
            if action == "cleanup_created_folder" {
                guard request["created_by_run"] as? Bool == true, let key = request["native_key"] as? String else { throw NotesTruthFailure.invalid }
                let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path
                var database: OpaquePointer?
                guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { throw NotesTruthFailure.invalid }
                defer { sqlite3_close(database) }
                var statement: OpaquePointer?
                guard sqlite3_prepare_v2(database, "SELECT Z_UUID,Z_PK FROM Z_METADATA,ZICCLOUDSYNCINGOBJECT WHERE ZIDENTIFIER=? AND Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICFolder')", -1, &statement, nil) == SQLITE_OK else { throw NotesTruthFailure.invalid }
                defer { sqlite3_finalize(statement) }
                sqlite3_bind_text(statement, 1, key, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
                if sqlite3_step(statement) == SQLITE_ROW {
                    let uuid = String(cString: sqlite3_column_text(statement, 0))
                    let id = "x-coredata://" + uuid + "/ICFolder/p" + String(sqlite3_column_int64(statement, 1))
                    let target = "folder id " + quoted(id)
                    _ = try run("if name of " + target + " is not \"Bridge Test\" then error number -2700\nrepeat with n in every note of " + target + "\nif name of n does not start with " + quoted(tag) + " then error number -2700\nend repeat\ndelete " + target)
                }
                return ["status":"verified"]
            }
            let folderResult = try run("return id of every folder whose name is \"Bridge Test\"")
            var folderID = folderResult.atIndex(1)?.stringValue ?? ""
            var created = false
            if folderID.isEmpty && action == "create" {
                folderID = try run("return id of (make new folder with properties {name:\"Bridge Test\"})").stringValue ?? ""
                created = true
            }
            if folderID.isEmpty { return ["status":"verified", "records":[]] }
            let target = "folder id " + quoted(folderID)
            if action == "create" {
                guard title.hasPrefix(tag) else { throw NotesTruthFailure.invalid }
                let body = request["html"] as? String ?? ("<div>" + title + "</div><div>synthetic body</div>")
                guard body.contains(tag) else { throw NotesTruthFailure.invalid }
                let id = try run("return id of (make new note at " + target + " with properties {body:" + quoted(body) + "})").stringValue ?? ""
                return ["status":"verified", "id":id, "folder_id":folderID, "folder_created":created, "effect_uptime_ns":clock_gettime_nsec_np(CLOCK_UPTIME_RAW)]
            }
            if action == "cleanup" || action == "delete" {
                _ = try run("repeat with n in (every note of " + target + ")\nif name of n starts with " + quoted(tag) + " then delete n\nend repeat")
            } else if action == "modify" {
                guard let id = request["id"] as? String, let replacement = request["plain_text"] as? String, replacement.hasPrefix(tag) else { throw NotesTruthFailure.invalid }
                let safe = try run("return name of note id " + quoted(id)).stringValue ?? ""
                guard safe.hasPrefix(tag) else { throw NotesTruthFailure.invalid }
                let html = replacement.replacingOccurrences(of:"&",with:"&amp;").replacingOccurrences(of:"<",with:"&lt;").components(separatedBy:"\n").map { "<div>" + $0 + "</div>" }.joined()
                _ = try run("set body of note id " + quoted(id) + " to " + quoted(html))
            } else { throw NotesTruthFailure.invalid }
            return ["status":"verified", "effect_uptime_ns":clock_gettime_nsec_np(CLOCK_UPTIME_RAW)]
        }
        guard verb == "observe" else { throw NotesTruthFailure.invalid }
        let selected = "every note"
        let value = try run("set output to {}\nrepeat with n in (" + selected + ")\nset locked to password protected of n\nset content to \"\"\nif not locked then set content to plaintext of n\nset end of output to {id of n, name of n, content, locked}\nend repeat\nreturn output")
        var records: [[String:Any]] = []
        if value.numberOfItems > 0 {
            for index in 1...value.numberOfItems {
                guard let item = value.atIndex(index) else { continue }
                let name = item.atIndex(2)?.stringValue ?? ""
                if !tag.isEmpty && !name.hasPrefix(tag) { continue }
                records.append(["id":item.atIndex(1)?.stringValue ?? "", "title":name, "plain_text":item.atIndex(3)?.stringValue ?? "", "locked":item.atIndex(4)?.booleanValue ?? false])
            }
        }
        return ["status":"verified", "records":records]
    }
}

private enum NotesTruthFailure: Error { case invalid, automation }
