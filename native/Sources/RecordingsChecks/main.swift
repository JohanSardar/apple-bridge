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
            if row["kind"].string == "absent" { rows.removeValue(forKey: row["collection"].string == "voice_memos.folder" ? "folder/" + row["native_key"].string : row["native_key"].string) }
            else { rows[row["collection"].string == "voice_memos.folder" ? "folder/" + row["native_key"].string : row["native_key"].string] = row["data"] }
        }
    }
    func matches(_ duration: Double) -> Bool { rows["fixture"]?["duration"].number == duration }
    func metadata(_ memo: Bool) -> Bool {
        guard let row = rows["fixture"] else { return false }
        let date = timestamp(Date(timeIntervalSinceReferenceDate: 800000000))
        if memo { return row["title"].string == "Synthetic" && row["date"].string == date && row["folder"].string == "1" && !row["recently_deleted"].bool }
        return row["sender"].string == "Synthetic" && row["date"].string == date && !row["read"].bool && row["mailbox"].string == "inbox" && row["type"].string == "carrierVoicemail"
    }
    func count() -> Int { rows.keys.filter { !$0.hasPrefix("folder/") }.count }
    func late() -> Bool { rows["fixture"]?["transcript"].array.count == 1 && rows["fixture"]?["audio_state"].string == "present" && rows["folder/1"]?["count"].number == 1 }
    func reference() -> JSON { rows["fixture"]?["audio"] ?? .null }

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
        if memo, CommandLine.arguments.contains("VMEMO-LATE-ASSETS"), let recording = source as? VoiceMemosSource {
            task.cancel()
            try execute("ALTER TABLE ZCLOUDRECORDING ADD COLUMN ZPATH TEXT; UPDATE ZCLOUDRECORDING SET ZPATH='fixture.m4a'; CREATE TABLE ZFOLDER(Z_PK INTEGER PRIMARY KEY,ZENCRYPTEDNAME TEXT); INSERT INTO ZFOLDER VALUES(1,'Synthetic')")
            try await recording.scan(into: sink, live: true)
            let lateTask = Task { try await recording.capture(.null, into: sink) }
            defer { lateTask.cancel() }
            try await Task.sleep(for: .milliseconds(100))
            let audio = root.appendingPathComponent("fixture.m4a")
            func atom(_ kind: String, _ payload: Data) -> Data {
                var size = UInt32(payload.count + 8).bigEndian
                return withUnsafeBytes(of: &size) { Data($0) } + Data(kind.utf8) + payload
            }
            try atom("mdat", Data(repeating: 0, count: 256)).write(to: audio)
            let audioDeadline = ContinuousClock.now
            while await sink.reference()["state"].string != "present" && audioDeadline.duration(to: .now) < .seconds(3) { try await Task.sleep(for: .milliseconds(10)) }
            let first = await sink.reference()
            try require(first["state"].string == "present")
            let payload = Data(#"{"attributedString":{"runs":["Synthetic",0],"attributeTable":[{"timeRange":[0,1.5]}]}}"#.utf8)
            let data = atom("mdat", Data(repeating: 0, count: 256)) + atom("moov", atom("trak", atom("udta", atom("tsrp", payload))))
            try data.write(to: audio)
            let transcriptDeadline = ContinuousClock.now
            while !(await sink.late()) && transcriptDeadline.duration(to: .now) < .seconds(3) { try await Task.sleep(for: .milliseconds(10)) }
            try require(await sink.late())
            lateTask.cancel()
            _ = try? await lateTask.value
            try require(await sink.count() == 1)
            let reference = await sink.reference()
            let chunk = try recording.readBlob("voice-memo-blob:" + reference["digest"].string, offset: 0, length: 131072)
            try require(Data(base64Encoded: chunk["bytes"].string) == data && chunk["eof"].bool)
            try await recording.scan(into: sink, live: true)
            try require(await sink.count() == 1)
            try execute("UPDATE ZCLOUDRECORDING SET ZPATH='../escape.m4a'")
            try await recording.scan(into: sink, live: true)
            try require(await sink.reference() == .null)
        }
        try execute("DELETE FROM " + table)
        if memo && CommandLine.arguments.contains("VMEMO-LATE-ASSETS"), let recording = source as? VoiceMemosSource { try await recording.scan(into: sink, live: true) }
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
