import Foundation
import SQLite3
import CryptoKit
import MessagesSource
import SourceSupport

private actor CommandCaptureSink: ObservationSink {
    var batches: [Set<String>] = []
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        let keys = Set(observations.filter { $0["collection"].string == "messages.message" && $0["kind"].string != "alias" }.map { $0["native_key"].string })
        if !keys.isEmpty { batches.append(keys) }
    }
}

enum CommandCaptureChecks {
    static func run() async throws -> Bool {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-command-capture-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let path = root.appendingPathComponent("chat.db").path
        let digest = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
        defer {
            for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: NSHomeDirectory() + "/Library/Application Support/AppleBridge/messages/" + digest + ".sqlite" + suffix) }
            try? FileManager.default.removeItem(at: root)
        }
        var database: OpaquePointer?
        guard sqlite3_open(path, &database) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_close(database) }
        let sql = "CREATE TABLE message(guid TEXT,text TEXT,is_from_me INTEGER,date INTEGER); CREATE TABLE chat(guid TEXT); CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER); CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER); CREATE TABLE attachment(guid TEXT,filename TEXT); CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER); INSERT INTO chat VALUES('self'); INSERT INTO message VALUES('first','[ab-test] first',1,100),('second','[ab-test] second',1,200); INSERT INTO chat_message_join VALUES(1,1),(1,2);"
        guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.invalid }
        let source = Messages(path: path, watcherEventsEnabled: false)
        let sink = CommandCaptureSink()
        let evidence = JSON.object(["native_key": .string("first"), "parts": .array([.object(["native_key": .string("first")]), .object(["native_key": .string("second")])])])
        try await source.captureCommand(evidence, into: sink)
        try await source.captureCommand(evidence, into: sink)
        let batches = await sink.batches
        return batches.count == 2 && batches.allSatisfy { $0 == Set(["first", "second"]) }
    }
}
