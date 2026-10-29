import Foundation
import SQLite3
import Darwin
import CallsSource
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
    func matches(_ duration: Double, answered: Bool) -> Bool {
        rows["1"]?["duration"].number == duration && rows["1"]?["answered"].bool == answered && rows["1"]?["direction"].string == (answered ? "incoming" : "missed") && rows["1"]?["service"].string == "cellular" && rows["1"]?["handle"].string == "synthetic" && rows["1"]?["name"].string == "Fixture"
    }
    func services() -> Bool { rows["2"]?["service"].string == "FaceTime video" && rows["3"]?["service"].string == "FaceTime audio" && rows["2"]?["direction"].string == "outgoing" }
    func count() -> Int { rows.count }
}

@main
struct Checks {
    static func require(_ value: Bool) throws { guard value else { throw SourceFailure.invalid } }
    @MainActor static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let path = root.appendingPathComponent("CallHistory.storedata").path
        var db: OpaquePointer?
        try require(sqlite3_open(path, &db) == SQLITE_OK)
        defer { sqlite3_close(db) }
        func execute(_ sql: String) throws { try require(sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK) }
        try execute("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE Z_METADATA(Z_VERSION INTEGER PRIMARY KEY,Z_UUID TEXT,Z_PLIST BLOB); INSERT INTO Z_METADATA VALUES(1,'synthetic',NULL); CREATE TABLE ZCALLRECORD(Z_PK INTEGER PRIMARY KEY,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER);")
        if CommandLine.arguments.contains("--dial") {
            let number = "+15555550122"
            func prepared(_ source: CallsSource) async throws -> Prepared {
                try await source.prepare(.object(["request_hash": .string("fixture"), "action": .object(["op": .string("phone.dial"), "args": .object(["number": .string(number)])])]))
            }
            let observed = CallsSource(path: path, observationWindow: 0.3, dialer: { url in
                guard url.absoluteString == "tel:+15555550122" else { throw SourceFailure.invalid }
                var connection: OpaquePointer?
                guard sqlite3_open(path, &connection) == SQLITE_OK else { throw SourceFailure.unavailable }
                defer { sqlite3_close(connection) }
                let sql = "INSERT INTO ZCALLRECORD VALUES(99,'+15555550122','Fixture',\(Date().timeIntervalSinceReferenceDate),0,1,0,'com.apple.telephony',1)"
                guard sqlite3_exec(connection, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
                return true
            })
            let outcome = try await observed.dispatch(prepared(observed))
            try require(outcome["observed"].bool && outcome["native_key"].string == "99")
            let missing = CallsSource(path: path, observationWindow: 0.2, dialer: { _ in
                var connection: OpaquePointer?
                guard sqlite3_open(path, &connection) == SQLITE_OK else { throw SourceFailure.unavailable }
                defer { sqlite3_close(connection) }
                let now = Date().timeIntervalSinceReferenceDate
                let sql = "INSERT INTO ZCALLRECORD VALUES(100,'+15555550122','Fixture',\(now),0,0,0,'com.apple.telephony',1); INSERT INTO ZCALLRECORD VALUES(101,'+15555550999','Fixture',\(now),0,1,0,'com.apple.telephony',1); INSERT INTO ZCALLRECORD VALUES(102,'+15555550122','Fixture',\(now - 60),0,1,0,'com.apple.telephony',1);"
                guard sqlite3_exec(connection, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
                return true
            })
            let unknown = try await missing.dispatch(prepared(missing))
            try require(unknown["disposition"].string == "indeterminate")
            let resolved = await missing.resolve("fixture", evidence: unknown)
            try require(resolved == .null)
            print("dial observed and indeterminate passed")
            return
        }
        let source = CallsSource(path: path, dialer: { _ in throw SourceFailure.invalid }), sink = Sink()
        let task = Task { try await source.capture(.null, into: sink) }
        defer { task.cancel() }
        try await Task.sleep(for: .milliseconds(100))
        if CommandLine.arguments.contains("--idle") {
            let emissions = await sink.emissions
            var before = rusage(), after = rusage()
            getrusage(RUSAGE_SELF, &before)
            try await Task.sleep(for: .seconds(300))
            getrusage(RUSAGE_SELF, &after)
            func seconds(_ usage: rusage) -> Double { Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1000000 }
            try require((seconds(after) - seconds(before)) / 300 * 100 < 2)
            try require(await sink.emissions == emissions)
            print("calls idle CPU passed")
            return
        }
        let start = ContinuousClock.now
        try execute("INSERT INTO ZCALLRECORD VALUES(1,'synthetic','Fixture',800000000,0,0,0,'com.apple.telephony',1)")
        while !(await sink.matches(0, answered: false)) && start.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.matches(0, answered: false))
        try require(start.duration(to: .now) < .seconds(1))
        try execute("UPDATE ZCALLRECORD SET ZDURATION=42,ZANSWERED=1 WHERE Z_PK=1")
        let update = ContinuousClock.now
        while !(await sink.matches(42, answered: true)) && update.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.matches(42, answered: true))
        try execute("INSERT INTO ZCALLRECORD VALUES(2,'synthetic','Fixture',800000000,5,1,1,'com.apple.facetime',8); INSERT INTO ZCALLRECORD VALUES(3,'synthetic','Fixture',800000000,5,0,1,'com.apple.facetime',16)")
        try await source.scan(into: sink, live: true)
        try require(await sink.services())
        do { try await source.scan(into: sink, live: true, maxRows: 0); throw SourceFailure.invalid } catch SourceFailure.unavailable {}
        try require(await sink.count() == 3)
        try execute("DELETE FROM ZCALLRECORD")
        let deleted = ContinuousClock.now
        while await sink.count() != 0 && deleted.duration(to: .now) < .seconds(1) { try await Task.sleep(for: .milliseconds(10)) }
        try require(await sink.count() == 0)
        print("calls fixture checks passed")
    }
}
