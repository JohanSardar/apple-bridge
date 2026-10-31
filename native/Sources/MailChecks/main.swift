import Foundation
import SQLite3
import MailSource
import SourceSupport

func require(_ value: Bool) throws { if !value { throw SourceFailure.corrupt } }
func execute(_ path: String, _ sql: String) throws {
    var db: OpaquePointer?
    guard sqlite3_open(path, &db) == SQLITE_OK else { throw SourceFailure.unavailable }
    defer { sqlite3_close(db) }
    guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.corrupt }
}
actor IdleSink: ObservationSink {
    var checkpoints = 0
    var messages = 0
    var failures = 0
    var firstRead = false
    var lastLive = false
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard completionEvidence == "complete" else { throw SourceFailure.corrupt }
        lastLive = live
        firstRead = observations.first { $0["collection"].string == "mail.message" && $0["native_key"].string == "1" }?["data"]["read"].bool ?? false
        checkpoints += 1
        messages = observations.filter { $0["collection"].string == "mail.message" && $0["kind"].string == "upsert" }.count
    }
    func failed() { failures += 1 }
}

@main
struct MailChecks {
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("Envelope Index").path
        try execute(path, "PRAGMA journal_mode=WAL; CREATE TABLE mailboxes(url TEXT); CREATE TABLE subjects(subject TEXT); CREATE TABLE addresses(address TEXT); CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT); INSERT INTO subjects VALUES('synthetic subject'); INSERT INTO addresses VALUES('sender@example.com'); INSERT INTO mailboxes VALUES('imap://one@example.com/INBOX'),('imap://two@example.com/Sent'),('imap://three@example.com/Archive');")
        for i in 1...1000 { try execute(path, "INSERT INTO messages VALUES(\(i % 3 + 1),1,1,1700000000,0,'<synthetic-\(i)@example.com>')") }
        let source = MailSource(path: path)
        let (generation, captured, complete) = try await source.snapshot()
        let records = captured.filter { $0["collection"].string == "mail.message" }
        try require(complete && records.count == 1000)
        try require(captured.filter { $0["collection"].string == "mail.account" }.count == 3)
        try require(captured.filter { $0["collection"].string == "mail.mailbox" }.count == 3)
        try require(Set(captured.map { $0["observed_at"].string }).count == 1)
        try require(records.allSatisfy { value in
            let mailbox = value["data"]["mailbox_ids"].array.first?.string
            let account = ["mailbox/1": "one@example.com", "mailbox/2": "two@example.com", "mailbox/3": "three@example.com"][mailbox ?? ""]
            return value["data"]["account_id"].string == account
        })
        try require(records.allSatisfy { $0["data"]["subject"].string == "synthetic subject" && $0["data"]["from"].string == "sender@example.com" && !$0["scoped_alias"].string.isEmpty })
        let scenario = CommandLine.arguments.last ?? ""
        if scenario == "MAIL-COMMANDS" {
 try await checkMailCommands()
 } else if scenario == "MAIL-BODY" {
            try await checkMailBodies(directory)
        } else if scenario == "MAIL-NO-FALSE-DELETE" {
            let (_, partial, done) = try await source.snapshot(maxRows: 10)
            try require(!done && partial.filter { $0["collection"].string == "mail.message" }.count == 10 && !partial.contains { $0["kind"].string == "absent" })
            try FileManager.default.moveItem(atPath: path, toPath: path + ".old")
            try execute(path, "CREATE TABLE mailboxes(url TEXT); CREATE TABLE subjects(subject TEXT); CREATE TABLE addresses(address TEXT); CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT); INSERT INTO mailboxes VALUES('imap://one@example.com/INBOX'); INSERT INTO subjects VALUES('synthetic subject'); INSERT INTO addresses VALUES('sender@example.com'); INSERT INTO messages(ROWID,mailbox,subject,sender,date_received,read,message_id) VALUES(2000,1,1,1,1700000000,0,'<synthetic-3@example.com>');")
            let (next, rebuilt, rebuiltDone) = try await source.snapshot()
            try require(rebuiltDone && next != generation && rebuilt.filter { $0["collection"].string == "mail.message" }.count == 1 && rebuilt.last!["scoped_alias"] == records[2]["scoped_alias"])
            try FileManager.default.moveItem(atPath: path, toPath: path + ".missing")
            do { _ = try await source.snapshot(); throw SourceFailure.corrupt } catch SourceFailure.unavailable {}
        } else if scenario == "MAIL-LIVE" {
            let watcher = DatabaseWatcher(path: path)
            defer { watcher.close() }
            let started = ProcessInfo.processInfo.systemUptime
            try execute(path, "UPDATE messages SET read=1,mailbox=2 WHERE ROWID=1; INSERT INTO messages VALUES(1,1,1,1700000000,0,'<new@example.com>');")
            for await _ in watcher.changes { break }
            let (_, updated, done) = try await source.snapshot()
            try require(done && updated.count == 1007 && updated[6]["data"]["read"].bool && updated[6]["data"]["mailbox_ids"].array == [.string("mailbox/2")])
            try require(ProcessInfo.processInfo.systemUptime - started < 1)
        } else if scenario == "MAIL-IDLE" {
            let sink = IdleSink()
            let capture = Task {
                do { try await source.capture(.null, into: sink) }
                catch is CancellationError {}
                catch { await sink.failed() }
            }
            defer { capture.cancel() }
            let deadline = ContinuousClock.now.advanced(by: .seconds(10))
            while await sink.checkpoints == 0 && ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(10))
            }
            try require(await sink.checkpoints == 1)
            try require(await sink.messages == 1000)
            try require(await sink.failures == 0)
            try require(await !sink.firstRead)
            try require(await !sink.lastLive)
            try await Task.sleep(for: .seconds(35))
            try require(await sink.checkpoints == 1)
            try require(await sink.failures == 0)
            try execute(path, "UPDATE messages SET read=1 WHERE ROWID=1")
            let changed = ContinuousClock.now.advanced(by: .seconds(10))
            while await sink.checkpoints == 1 && ContinuousClock.now < changed {
                try await Task.sleep(for: .milliseconds(10))
            }
            try require(await sink.checkpoints == 2)
            try require(await sink.messages == 1000)
            try require(await sink.failures == 0)
            try require(await sink.firstRead)
            try require(await sink.lastLive)
            try await Task.sleep(for: .seconds(2))
            try require(await sink.checkpoints == 2)
            try require(await sink.failures == 0)
            capture.cancel()
            await capture.value
        }
        let encoded = try JSONEncoder().encode(JSON.object(["observations": .array(captured)]))
        print(String(decoding: encoded, as: UTF8.self))
    }
}
