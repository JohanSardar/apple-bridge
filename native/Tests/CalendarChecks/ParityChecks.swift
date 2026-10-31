import Foundation
import SQLite3
import CalendarSource
import AgendaGate
import SourceSupport

@AgendaActor
enum CalendarParityChecks {
    static func run() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-calendar-parity-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let path = root.appendingPathComponent("Calendar.sqlitedb")
        var database: OpaquePointer?
        guard sqlite3_open(path.path, &database) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_close(database) }
        func execute(_ sql: String) throws {
            guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        }
        try execute("CREATE TABLE Calendar(ROWID INTEGER PRIMARY KEY,UUID TEXT); CREATE TABLE Location(ROWID INTEGER PRIMARY KEY,title TEXT,address TEXT,latitude INTEGER,longitude INTEGER,radius INTEGER); CREATE TABLE Participant(ROWID INTEGER PRIMARY KEY,status INTEGER,owner_id INTEGER,address TEXT,role TEXT,is_self INTEGER); CREATE TABLE Conference(owner_id INTEGER,url TEXT); CREATE TABLE CalendarItem(ROWID INTEGER PRIMARY KEY,unique_identifier TEXT,UUID TEXT,external_id TEXT,calendar_id INTEGER,location_id INTEGER,self_attendee_id INTEGER,travel_time INTEGER,travel_advisory_behavior INTEGER,invitation_status INTEGER,privacy_level INTEGER,priority INTEGER,conference_url TEXT); CREATE TABLE Attachment(owner_id INTEGER,file_id INTEGER); CREATE TABLE AttachmentFile(ROWID INTEGER PRIMARY KEY,UUID TEXT,filename TEXT,local_path TEXT,file_size INTEGER,format TEXT); INSERT INTO Calendar VALUES(1,'[ab-test] calendar'); INSERT INTO Participant VALUES(1,1,1,'mailto:self@ab-test.invalid','attendee',1),(2,2,1,'mailto:organizer@ab-test.invalid','organizer',0),(3,1,2,'mailto:self@ab-test.invalid','attendee',1); INSERT INTO CalendarItem VALUES(1,'[ab-test] event','[ab-test] uuid','[ab-test] external',1,NULL,1,NULL,0,1,0,0,NULL),(2,'[ab-test] untouched event','[ab-test] untouched uuid','[ab-test] untouched external',1,NULL,3,0,0,1,0,0,NULL);")
        let observation: [String: JSON] = ["event": .object(["collection": .string("calendar.event"), "native_key": .string("[ab-test] event"), "data": .object(["calendar_id": .object(["$ref": .object(["native_key": .string("[ab-test] calendar")])])])])]
        let reader = CalendarStoreEnrichment(root: root)
        let unknown = await reader.enrich(observation, lookups: [:], force: true)
        guard unknown["event"]?["data"]["store_coverage"]["travel_time"]["status"] != .string("complete") else { throw SourceFailure.invalid }
        try execute("UPDATE CalendarItem SET travel_time='unknown' WHERE ROWID=1")
        let invalid = await reader.enrich(observation, lookups: [:], force: true)
        guard invalid["event"]?["data"]["store_coverage"]["travel_time"]["status"] == .string("unsupported") else { throw SourceFailure.invalid }
        try execute("UPDATE CalendarItem SET travel_time=0 WHERE ROWID=1")
        let zero = await reader.enrich(observation, lookups: [:], force: true)
        guard zero["event"]?["data"]["travel_time"] == .number(0), zero["event"]?["data"]["store_coverage"]["travel_time"]["status"] == .string("complete") else { throw SourceFailure.invalid }
        for status in [2,4,3,2] {
            try execute("UPDATE Participant SET status=\(status) WHERE ROWID=1")
            let row = try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] event", external: "", calendar: "[ab-test] calendar", detached: false)
            let invitation = await reader.enrich(observation, lookups: [:], force: true)
            let untouched = try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] untouched event", external: "", calendar: "[ab-test] calendar", detached: false)
            guard row?["self_status"]?.number == Double(status), row?["invitation_status"]?.number == 1,
                  invitation["event"]?["data"]["owner_participation_status"].number == Double(status),
                  invitation["event"]?["data"]["invitation_status"].number == 1,
                  untouched?["self_status"]?.number == 1 else { throw SourceFailure.invalid }
        }
        try execute("UPDATE CalendarItem SET travel_time=900 WHERE ROWID=1")
        let changed = try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] event", external: "", calendar: "[ab-test] calendar", detached: false)
        guard changed?["travel_time"]?.number == 900 else { throw SourceFailure.invalid }
        let unverified = await reader.enrich(observation, lookups: [:], force: true)
        guard unverified["event"]?["data"]["travel_time"] == .null, unverified["event"]?["data"]["store_coverage"]["travel_time"]["status"] == .string("unsupported") else { throw SourceFailure.invalid }
        try Data("[ab-test] attachment".utf8).write(to: root.appendingPathComponent("fixture.txt"))
        try execute("INSERT INTO AttachmentFile VALUES(1,'[ab-test] file','fixture.txt','fixture.txt',20,'text/plain'); INSERT INTO Attachment VALUES(1,1)")
        let attached = await reader.enrich(observation, lookups: [:], force: true)
        guard attached["event"]?["data"]["attachments"].array.count == 1 else { throw SourceFailure.invalid }
        try execute("DELETE FROM Attachment WHERE owner_id=1")
        let removed = await reader.enrich(observation, lookups: [:], force: true)
        guard removed["event"]?["data"]["attachments"] == .array([]) else { throw SourceFailure.invalid }
        try execute("DELETE FROM AttachmentFile; DELETE FROM Participant; DELETE FROM CalendarItem; DELETE FROM Calendar")
        guard try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] event", external: "", calendar: "[ab-test] calendar", detached: false) == nil else { throw SourceFailure.invalid }
        print("calendar invitation response checks passed")
        print("calendar parity checks passed")
    }
}
