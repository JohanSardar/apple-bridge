import Foundation
import SourceSupport

@MainActor
public final class NotesSource: Source {
    public nonisolated let name = "notes"
    public static let operations = ["notes.create", "notes.update", "notes.delete", "notes.create_folder", "notes.move", "notes.rename_folder"]
    public init() {}
    public func primitive(_ operation: String, args request: JSON) async throws -> JSON {
        guard operation == "notes.write", Self.operations.contains(request["op"].string) else { throw SourceFailure.unsupported }
        let quote = NotesScript.quote
        let note = request["note_id"].string, folder = request["folder_id"].string
        var scriptID = note
        var failure = JSON.null
        switch request["op"].string {
        case "notes.rename_folder":
            scriptID = folder
            do { try await NotesFolderUI.rename(id: folder, from: request["old_title"].string, to: request["title"].string) }
            catch { failure = .string((error as? SourceFailure).map { String(describing: $0) } ?? "unavailable") }
        case "notes.create_folder":
            scriptID = try NotesScript.run("return id of (make new folder with properties {name:" + quote(request["title"].string) + "})")
        case "notes.create":
            scriptID = try NotesScript.run("return id of (make new note at folder id " + quote(folder) + " with properties {body:" + quote(request["html"].string) + "})")
        case "notes.move":
            _ = try NotesScript.run("move note id " + quote(note) + " to folder id " + quote(folder))
        case "notes.delete":
            _ = try NotesScript.run(NotesScript.deleteNote(note, folderID: folder, trashFolderID: request["trash_folder_id"].string.isEmpty ? nil : request["trash_folder_id"].string))
        default:
            _ = try NotesScript.run("set body of note id " + quote(note) + " to " + quote(request["html"].string))
        }
        return .object(["script_id": .string(scriptID), "failure": failure])
    }
}
