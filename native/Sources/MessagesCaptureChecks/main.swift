import Foundation
import SQLite3
import MessagesSource
import SourceSupport
import Darwin
import CryptoKit

final class SyntheticDatabase {
    private var connection: OpaquePointer?
    init(_ path: String) throws {
        guard sqlite3_open(path, &connection) == SQLITE_OK else { throw SourceFailure.unavailable }
    }
    deinit { sqlite3_close(connection) }
    func execute(_ sql: String) throws {
        guard sqlite3_exec(connection, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.invalid }
    }
}

actor CaptureSink: ObservationSink {
    private var records: [String: JSON] = [:]
    private var generations: Set<String> = []
    private var complete = false
    private var failed = false
    private let hookPath: String?
    private var referenceStage = 0
    init(hookPath: String? = nil) { self.hookPath = hookPath }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        if state != "ready" { failed = true }
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        generations.insert(generation)
        if observations.contains(where: { $0["kind"].string == "absent" && ["retained-message", "retained-chat"].contains($0["native_key"].string) }) { failed = true }
        for item in observations where item["kind"].string == "upsert" { records[item["native_key"].string] = item["data"] }
        if completionEvidence == "complete" { complete = true }
        if live, let hookPath {
            if referenceStage == 0 && observations.contains(where: { $0["collection"].string == "messages.chat" && $0["data"]["display_name"].string == "changed" }) {
                referenceStage = 1
                let database = try SyntheticDatabase(hookPath)
                try database.execute("UPDATE message SET text='newer' WHERE guid='synthetic-000000'; INSERT INTO message VALUES('synthetic-reference-one','synthetic','synthetic-000000');")
            } else if referenceStage == 1 && observations.contains(where: { $0["native_key"].string == "synthetic-000000" && $0["data"]["text"].string == "newer" }) {
                referenceStage = 2
                let database = try SyntheticDatabase(hookPath)
                try database.execute("UPDATE message SET text='newest' WHERE guid='synthetic-000001'; INSERT INTO message VALUES('synthetic-reference-two','synthetic','synthetic-000001');")
            }
        }
    }
    func recoverable(_ key: String) -> Bool { !failed && records[key]?["deleted_at"] != nil && records[key]?["deleted_at"] != .null }
    func record(_ key: String) -> JSON { records[key] ?? .null }
    func contains(_ key: String) -> Bool { records[key] != nil && !failed }
    func completed() -> Bool { complete && !failed }
    func referencesFresh() -> Bool { records["synthetic-000000"]?["text"].string == "newer" && records["synthetic-000001"]?["text"].string == "newest" && !failed }
    func replaced() -> Bool { generations.count > 1 && !failed }
}

@main
struct MessagesCaptureChecks {
    static func require(_ value: Bool, _ name: String) throws {
        guard value else {
            try FileHandle.standardError.write(contentsOf: Data(("Messages capture checks failed: " + name + "\n").utf8))
            throw SourceFailure.invalid
        }
    }
    static func wait(_ seconds: Double = 5, _ condition: @Sendable () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .milliseconds(Int64(seconds * 1000))
        while ContinuousClock.now < deadline {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return false
    }
    static func main() async throws {
        if CommandLine.arguments.contains("--rich") { try await RichCaptureChecks.run(); return }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("messages-capture-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer {
            for name in ["events/chat.db", "poll/chat.db", "replace.db", "references.db", "visibility.db"] {
                let path = root.appendingPathComponent(name).path
                let digest = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
                let baseline = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridge/messages/" + digest + ".sqlite").path
                for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: baseline + suffix) }
            }
            try? FileManager.default.removeItem(at: root)
        }
        let seed = root.appendingPathComponent("seed.db").path
        do {
            let database = try SyntheticDatabase(seed)
            try database.execute("CREATE TABLE message(guid TEXT,text TEXT); INSERT INTO message VALUES('synthetic-initial','synthetic');")
        }
        for events in [true, false] {
            let directory = root.appendingPathComponent(events ? "events" : "poll")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let path = directory.appendingPathComponent("chat.db").path
            try FileManager.default.copyItem(atPath: seed, toPath: path)
            if !events {
                let partial = try SyntheticDatabase(path)
                try partial.execute("CREATE TABLE chat(guid TEXT); CREATE TABLE handle(id TEXT); CREATE TABLE chat_handle_join(chat_id INTEGER); CREATE TABLE chat_message_join(message_id INTEGER); CREATE TABLE attachment(guid TEXT,filename TEXT); CREATE TABLE message_attachment_join(attachment_id INTEGER); INSERT INTO chat VALUES('synthetic-chat'); INSERT INTO attachment VALUES('synthetic-attachment','');")
            }
            let source = Messages(path: path, watcherEventsEnabled: events)
            let readiness = await source.open()
            try require(readiness.state == "ready" && readiness.coverage["fields"].string == "partial" && readiness.coverage["missing"].array.contains(.string("message.handle_id")), "partial schema coverage")
            let sink = CaptureSink()
            let task = Task { try await source.capture(.null, into: sink) }
            try require(await wait { let complete = await sink.completed(); let contains = await sink.contains("synthetic-initial"); return complete && contains }, "partial schema capture")
            do {
                let database = try SyntheticDatabase(path)
                try database.execute("PRAGMA journal_mode=WAL; INSERT INTO message VALUES('synthetic-wal','synthetic');")
                try require(await wait(events ? 1.5 : 4) { await sink.contains("synthetic-wal") }, events ? "native WAL event" : "disarmed events polling fallback")
                try database.execute("PRAGMA wal_checkpoint(TRUNCATE);")
            }
            if FileManager.default.fileExists(atPath: path + "-wal") {
                try FileManager.default.moveItem(atPath: path + "-wal", toPath: path + "-wal-retired")
            }
            do {
                let database = try SyntheticDatabase(path)
                try database.execute("INSERT INTO message VALUES('synthetic-rotated','synthetic');")
                try require(await wait(events ? 1.5 : 4) { await sink.contains("synthetic-rotated") }, "WAL inode rotation capture")
                try database.execute("INSERT INTO message VALUES('synthetic-after-rotation','synthetic');")
                try require(await wait(events ? 1.5 : 4) { await sink.contains("synthetic-after-rotation") }, "WAL event after rearm")
            }
            task.cancel()
            _ = try? await task.value
        }
        let path = root.appendingPathComponent("replace.db").path
        try FileManager.default.copyItem(atPath: seed, toPath: path)
        let source = Messages(path: path)
        let sink = CaptureSink()
        let task = Task { try await source.capture(.null, into: sink) }
        try require(await wait { await sink.completed() }, "replacement initial capture")
        let replacement = root.appendingPathComponent("replacement.db").path
        try FileManager.default.copyItem(atPath: seed, toPath: replacement)
        do {
            let database = try SyntheticDatabase(replacement)
            try database.execute("INSERT INTO message VALUES('synthetic-replaced','synthetic');")
        }
        guard Darwin.rename(replacement, path) == 0 else { throw SourceFailure.unavailable }
        try require(await wait(1.5) { let contains = await sink.contains("synthetic-replaced"); let replaced = await sink.replaced(); return contains && replaced }, "native database replacement rearm")
        do {
            let database = try SyntheticDatabase(path)
            try database.execute("INSERT INTO message VALUES('synthetic-after-rearm','synthetic');")
        }
        try require(await wait(1.5) { await sink.contains("synthetic-after-rearm") }, "native event after replacement")
        task.cancel()
        _ = try? await task.value
        let referencePath = root.appendingPathComponent("references.db").path
        let database = try SyntheticDatabase(referencePath)
        try database.execute("CREATE TABLE message(guid TEXT,text TEXT,reply_to_guid TEXT); CREATE TABLE chat(guid TEXT,display_name TEXT); INSERT INTO chat VALUES('synthetic-chat','initial');")
        for index in 0..<2200 { try database.execute("INSERT INTO message VALUES('synthetic-" + String(format: "%06d", index) + "','initial',NULL);") }
        let referenceSource = Messages(path: referencePath)
        let referenceSink = CaptureSink(hookPath: referencePath)
        let referenceTask = Task { try await referenceSource.capture(.null, into: referenceSink) }
        try require(await wait(20) { await referenceSink.completed() }, "referenced row initial capture")
        try database.execute("UPDATE chat SET display_name='changed';")
        try require(await wait(5) { await referenceSink.referencesFresh() }, "every insert drain processes references until quiescent")
        referenceTask.cancel()
        _ = try? await referenceTask.value
        let visibilityPath = root.appendingPathComponent("visibility.db").path
        let visibilityDatabase = try SyntheticDatabase(visibilityPath)
        try visibilityDatabase.execute("CREATE TABLE message(guid TEXT,text TEXT); CREATE TABLE chat(guid TEXT); CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER); CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date REAL); INSERT INTO message VALUES('retained-message','[ab-test synthetic] retained'); INSERT INTO chat VALUES('retained-chat'); INSERT INTO chat_message_join VALUES(1,1);")
        let visibility = Messages(path: visibilityPath)
        let visibilitySink = CaptureSink()
        let visibilityTask = Task { try await visibility.capture(.null, into: visibilitySink) }
        try require(await wait { await visibilitySink.completed() }, "visibility initial capture")
        try visibilityDatabase.execute("INSERT INTO chat_recoverable_message_join VALUES(1,1,800000000000000000);")
        try require(await wait(4) {
            let message = await visibilitySink.recoverable("retained-message")
            let chat = await visibilitySink.recoverable("retained-chat")
            return message && chat
        }, "recoverable delete retains message and chat")
        try visibilityDatabase.execute("DELETE FROM chat_recoverable_message_join;")
        try require(await wait(4) {
            let present = await visibilitySink.contains("retained-message")
            let deleted = await visibilitySink.recoverable("retained-message")
            return present && !deleted
        }, "restoring membership clears deleted state")
        visibilityTask.cancel()
        _ = try? await visibilityTask.value
        print("all Messages capture checks passed")
    }
}
