#if LAB
import Foundation
import SourceSupport

@MainActor
public enum NotesExtrasProbe {
    public static func run() async -> JSON {
        let tag = "[ab-test extras-" + UUID().uuidString + "]"
        var folders: [String] = []
        var note = ""
        var reason = ""
        var assertions: [String: JSON] = [:]
        let store = NotesStore(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path)
        do {
            for suffix in ["a", "b"] {
                folders.append(try NotesScript.run("return id of (make new folder with properties {name:" + NotesScript.quote("Bridge Test " + tag + suffix) + "})"))
            }
            note = try NotesScript.run("return id of (make new note at folder id " + NotesScript.quote(folders[0]) + " with properties {body:" + NotesScript.quote(NotesScript.html(tag + " note")) + "})")
            _ = try NotesScript.run("set name of folder id " + NotesScript.quote(folders[1]) + " to " + NotesScript.quote("Bridge Test " + tag + " renamed"))
            _ = try NotesScript.run("move note id " + NotesScript.quote(note) + " to folder id " + NotesScript.quote(folders[1]))
            for _ in 0..<50 {
                let rows = try store.read().observations
                if let folder = rows.first(where: { $0["scoped_alias"].string == folders[1] }),
                   let item = rows.first(where: { $0["scoped_alias"].string == note }),
                   item["data"]["folder_id"]["$ref"]["native_key"] == folder["native_key"] {
                    assertions["move_store_readback"] = .bool(true)
                    assertions["rename_store_readback"] = .bool(folder["data"]["title"].string == "Bridge Test " + tag + " renamed")
                    break
                }
                try await Task.sleep(for: .milliseconds(100))
            }
        } catch { reason = String(describing: error); assertions["tagged_mutation"] = .bool(false) }
        do {
            if !note.isEmpty {
                guard try NotesScript.run("return name of note id " + NotesScript.quote(note)).hasPrefix(tag) else { throw SourceFailure.conflict }
                _ = try NotesScript.run("delete note id " + NotesScript.quote(note))
            }
            for folder in folders {
                guard try NotesScript.run("return name of folder id " + NotesScript.quote(folder)).hasPrefix("Bridge Test " + tag),
                      try NotesScript.run("return count of notes of folder id " + NotesScript.quote(folder)) == "0" else { throw SourceFailure.conflict }
                _ = try NotesScript.run("delete folder id " + NotesScript.quote(folder))
            }
            var clean = false
            for _ in 0..<50 {
                let rows = try store.read().observations
                clean = !rows.contains { (folders.contains($0["scoped_alias"].string) || $0["scoped_alias"].string == note) && !$0["data"]["recently_deleted"].bool }
                if clean { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            assertions["owned_cleanup"] = .bool(clean)
        } catch { assertions["owned_cleanup"] = .bool(false) }
        let passed = assertions["move_store_readback"]?.bool == true && assertions["rename_store_readback"]?.bool == true && assertions["owned_cleanup"]?.bool == true
        return .object(["reason": .string(reason), "assertions": .object(assertions), "passed": .bool(passed)])
    }
}
#endif
