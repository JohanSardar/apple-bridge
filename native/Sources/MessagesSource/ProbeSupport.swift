import Foundation
import CryptoKit
import AppKit
import ApplicationServices

public enum ProbeFailure: Error {
    case invalidArguments, unavailable, unsafeReport, noSelfHandle
}

public enum PrivateReport {
    public static var directory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev", isDirectory: true)
    }

    public static func validate(_ path: String) throws -> URL {
        let url = URL(fileURLWithPath: path).standardizedFileURL
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        guard url.deletingLastPathComponent().resolvingSymlinksInPath() == directory.resolvingSymlinksInPath(),
              url.lastPathComponent != ".", url.lastPathComponent != "..",
              !FileManager.default.fileExists(atPath: url.path) || url.resolvingSymlinksInPath() == url else {
            throw ProbeFailure.unsafeReport
        }
        return url
    }

    public static func write(_ object: Any, to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    public static func redact(_ text: String) -> [String: Any] {
        ["length": text.count, "sha256": SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined()]
    }
}

public enum MessagesAppleScript {
    public static func literal(_ text: String) -> String {
        "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\n", with: "\\n") + "\""
    }

    public static func execute(_ body: String) -> Int {
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 3 seconds\n" + body + "\nend timeout") else { return -1 }
        _ = script.executeAndReturnError(&error)
        return (error?[NSAppleScript.errorNumber] as? Int) ?? 0
    }
}
