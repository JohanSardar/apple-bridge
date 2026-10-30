import Foundation
import CryptoKit
import SQLite3
import AppKit
import ApplicationServices

@MainActor
public enum ProbeApplications {
    public static func open(_ bundle: String) async throws {
        guard NSRunningApplication.runningApplications(withBundleIdentifier: bundle).isEmpty else { return }
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { throw ProbeFailure.unavailable }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        configuration.addsToRecentItems = false
        _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
    }
}

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

public final class MessagesDatabase {
    private var connection: OpaquePointer?

    public init() throws {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        guard sqlite3_open_v2(path, &connection, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            if let connection { sqlite3_close(connection) }
            connection = nil
            throw ProbeFailure.unavailable
        }
        sqlite3_busy_timeout(connection, 200)
        _ = try rows("SELECT ROWID FROM message LIMIT 1")
    }

    deinit { sqlite3_close(connection) }

    public func rows(_ sql: String, bindings: [String] = []) throws -> [[String: String]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(connection, sql, -1, &statement, nil) == SQLITE_OK else { throw ProbeFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        for (index, value) in bindings.enumerated() {
            let result = value.withCString { sqlite3_bind_text(statement, Int32(index + 1), $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            guard result == SQLITE_OK else { throw ProbeFailure.unavailable }
        }
        var result: [[String: String]] = []
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return result }
            guard status == SQLITE_ROW else { throw ProbeFailure.unavailable }
            var row: [String: String] = [:]
            for index in 0..<sqlite3_column_count(statement) {
                if let value = sqlite3_column_text(statement, index) {
                    row[String(cString: sqlite3_column_name(statement, index))] = String(cString: value)
                }
            }
            result.append(row)
        }
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
