import Foundation
import SQLite3
import Darwin
import VoiceMemosSource
import VoicemailSource
import SourceSupport

actor Sink: ObservationSink {
    var rows: [String: JSON] = [:]
    var emissions = 0
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        emissions += 1
        for row in observations {
            if row["kind"].string == "absent" { rows.removeValue(forKey: row["native_key"].string) }
            else { rows[row["native_key"].string] = row["data"] }
        }
    }
    func matches(_ duration: Double) -> Bool { rows["fixture"]?["duration"].number == duration }
    func metadata(_ memo: Bool) -> Bool {
        guard let row = rows["fixture"] else { return false }
        let date = timestamp(Date(timeIntervalSinceReferenceDate: 800000000))
        if memo { return row["title"].string == "Synthetic" && row["date"].string == date && row["folder"].string == "1" && !row["recently_deleted"].bool }
        return row["sender"].string == "Synthetic" && row["date"].string == date && !row["read"].bool && row["mailbox"].string == "inbox" && row["type"].string == "carrierVoicemail"
    }
    func count() -> Int { rows.count }
}

@main
struct Checks {
    static func require(_ value: Bool) throws { guard value else { throw SourceFailure.invalid } }
    @MainActor static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        for memo in [true, false] {
        let path = root.appendingPathComponent(memo ? "CloudRecordings.db" : "voicemail.db").path
        var db: OpaquePointer?
        try require(sqlite3_open(path, &db) == SQLITE_OK)
        defer { sqlite3_close(db) }
        func execute(_ sql: String) throws { try require(sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK) }
        try execute("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE Z_METADATA(Z_UUID TEXT); INSERT INTO Z_METADATA VALUES('fixture');")
        let table = memo ? "ZCLOUDRECORDING" : "ZSTOREDMESSAGE"
        try execute(memo ? "CREATE TABLE ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER)" : "CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER)")
        let source: any Source = memo ? VoiceMemosSource(path: path) : VoicemailSource(path: path)
        let sink = Sink()
        let task = Task { try await source.capture(.null, into: sink) }
        defer { task.cancel() }
        try await Task.sleep(for: .milliseconds(100))
        try execute(memo ? "INSERT INTO ZCLOUDRECORDING VALUES(1,'fixture','Synthetic',800000000,0,NULL,1)" : "INSERT INTO ZSTOREDMESSAGE VALUES(1,'fixture','Synthetic',800000000,0,0,0,'com.apple.coretelephony',0)")
        let start = ContinuousClock.now
        while !(await sink.matches(0)) && start.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.matches(0))
        try require(await sink.metadata(memo))
        try execute("UPDATE " + table + " SET ZDURATION=42")
        let update = ContinuousClock.now
        while !(await sink.matches(42)) && update.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.matches(42))
        try require(await sink.count() == 1)
        do {
            if let recording = source as? VoiceMemosSource { try await recording.scan(into: sink, live: true, maxRows: 0) }
            if let message = source as? VoicemailSource { try await message.scan(into: sink, live: true, maxRows: 0) }
            throw SourceFailure.invalid
        } catch SourceFailure.unavailable {}
        try require(await sink.count() == 1)
        try execute("DELETE FROM " + table)
        let deleted = ContinuousClock.now
        while await sink.count() != 0 && deleted.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.count() == 0)
        task.cancel()
        }
        let missing = VoiceMemosSource(path: root.appendingPathComponent("missing").path)
        try require(await missing.open().state == "unavailable")
        let missingMessage = VoicemailSource(path: root.appendingPathComponent("missing-message").path)
        try require(await missingMessage.open().state == "unavailable")
        print("metadata fixture checks passed")
    }
}
