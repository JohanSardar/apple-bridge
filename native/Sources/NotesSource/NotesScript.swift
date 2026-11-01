import Foundation
import Carbon
import SourceSupport

@MainActor
public enum NotesScript {
    public static func quote(_ value: String) -> String {
        "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\n", with: "\\n") + "\""
    }
    public static func run(_ body: String, trace: CaptureTrace? = nil) throws -> String {
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.Notes")
        guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { throw SourceFailure.permissionMissing }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Notes\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        trace?.record("notes_script_completed", fields: ["source": .string("notes"), "apple_event_error": .number(Double(error?["NSAppleScriptErrorNumber"] as? Int ?? 0))])
        guard error == nil else { throw SourceFailure.unavailable }
        return result.stringValue ?? ""
    }
    public static func html(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;").components(separatedBy: "\n").map { "<div>" + ($0.isEmpty ? "<br>" : $0) + "</div>" }.joined()
    }
    public static func deleteNote(_ id: String, folderID: String, trashFolderID: String? = nil) -> String {
        let note = quote(id), folder = quote(folderID)
        let removal = trashFolderID.map { "move targetNote to folder id " + quote($0) } ?? "delete targetNote"
        return "set matches to (every note of folder id " + folder + " whose id is " + note + ")\nif (count of matches) is not 1 then error \"Note could not be found in its folder.\"\nset targetNote to item 1 of matches\nif (id of targetNote) is not " + note + " then error \"Note identity changed.\"\n" + removal
    }
}
