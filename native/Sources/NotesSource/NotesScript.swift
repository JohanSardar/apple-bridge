import Foundation
import SourceSupport

@MainActor
public enum NotesScript {
    public static func quote(_ value: String) -> String {
        "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\n", with: "\\n") + "\""
    }
    public static func run(_ body: String) throws -> String {
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application \"Notes\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result.stringValue ?? ""
    }
    public static func html(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;").components(separatedBy: "\n").map { "<div>" + ($0.isEmpty ? "<br>" : $0) + "</div>" }.joined()
    }
}
