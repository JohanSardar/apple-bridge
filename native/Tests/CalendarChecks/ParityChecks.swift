import Foundation
import SQLite3
import EventKit
import CoreLocation
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
        try execute("CREATE TABLE Calendar(ROWID INTEGER PRIMARY KEY,UUID TEXT); CREATE TABLE Location(ROWID INTEGER PRIMARY KEY,title TEXT,address TEXT,latitude INTEGER,longitude INTEGER,radius INTEGER); CREATE TABLE Participant(ROWID INTEGER PRIMARY KEY,status INTEGER); CREATE TABLE Conference(owner_id INTEGER,url TEXT); CREATE TABLE CalendarItem(ROWID INTEGER PRIMARY KEY,unique_identifier TEXT,UUID TEXT,external_id TEXT,calendar_id INTEGER,location_id INTEGER,self_attendee_id INTEGER,travel_time INTEGER,travel_advisory_behavior INTEGER,invitation_status INTEGER,privacy_level INTEGER,priority INTEGER,conference_url TEXT); CREATE TABLE Attachment(owner_id INTEGER,file_id INTEGER); CREATE TABLE AttachmentFile(ROWID INTEGER PRIMARY KEY,UUID TEXT,filename TEXT,local_path TEXT,file_size INTEGER,format TEXT); INSERT INTO Calendar VALUES(1,'[ab-test] calendar'); INSERT INTO Participant VALUES(1,1); INSERT INTO CalendarItem VALUES(1,'[ab-test] event','[ab-test] uuid','[ab-test] external',1,NULL,1,NULL,0,0,0,0,NULL);")
        let observation: [String: JSON] = ["event": .object(["collection": .string("calendar.event"), "native_key": .string("[ab-test] event"), "data": .object(["calendar_id": .object(["$ref": .object(["native_key": .string("[ab-test] calendar")])])])])]
        let location = EKStructuredLocation(title: "[ab-test] location")
        location.geoLocation = CLLocation(latitude: 41.88, longitude: -87.63)
        location.radius = 100
        let native = CalendarEventKitFields.structuredLocation(location)
        guard native["title"] == .string("[ab-test] location"), native["latitude"] == .number(41.88), native["longitude"] == .number(-87.63), native["radius"] == .number(100), CalendarEventKitFields.structuredLocation(nil) == .null, CalendarEventKitFields.participants(nil) == .array([]) else { throw SourceFailure.invalid }
        guard CalendarEventKitFields.ownerParticipationStatus(nil) == .null else { throw SourceFailure.invalid }
        let now = Date(timeIntervalSince1970: 1793448000)
        let identityWindows = CalendarEventKitFields.identitySearchIntervals(around: now)
        guard identityWindows.contains(where: { $0.contains(now.addingTimeInterval(-2 * 365 * 86400)) }),
              identityWindows.contains(where: { $0.contains(now.addingTimeInterval(180 * 86400)) }),
              identityWindows.allSatisfy({ $0.duration > 0 && $0.duration < 4 * 365 * 86400 }),
              zip(identityWindows, identityWindows.dropFirst()).allSatisfy({ $0.end == $1.start }) else { throw SourceFailure.invalid }
        let reader = CalendarStoreEnrichment(root: root)
        var nativeObservation = observation["event"]!.object
        var nativeData = nativeObservation["data"]!.object
        nativeData["structured_location"] = native
        nativeData["owner_participation_status"] = .number(4)
        nativeObservation["data"] = .object(nativeData)
        try execute("INSERT INTO Location VALUES(1,'[ab-test] store location','[ab-test] address',41880000,-87630000,100); UPDATE CalendarItem SET location_id=1 WHERE ROWID=1")
        let projected = await reader.enrich(["event": .object(nativeObservation)], lookups: [:], force: true)
        guard projected["event"]?["data"]["owner_participation_status"] == .number(4) else { throw SourceFailure.invalid }
        guard projected["event"]?["data"]["structured_location"]["latitude"] == .number(41.88), projected["event"]?["data"]["structured_location"]["longitude"] == .number(-87.63), projected["event"]?["data"]["structured_location"]["address"] == .string("[ab-test] address"), projected["event"]?["data"]["store_coverage"]["structured_location"]["status"] == .string("complete") else { throw SourceFailure.invalid }
        let unavailable = CalendarStoreEnrichment(root: root.appendingPathComponent("missing"))
        let fallback = await unavailable.enrich(["event": .object(nativeObservation)], lookups: [:], force: true)
        guard fallback["event"]?["data"]["structured_location"] == native else { throw SourceFailure.invalid }
        guard fallback["event"]?["data"]["owner_participation_status"] == .number(4) else { throw SourceFailure.invalid }

        let unknown = await reader.enrich(observation, lookups: [:], force: true)
        guard unknown["event"]?["data"]["store_coverage"]["travel_time"]["status"] != .string("complete") else { throw SourceFailure.invalid }
        try execute("UPDATE CalendarItem SET travel_time='unknown' WHERE ROWID=1")
        let invalid = await reader.enrich(observation, lookups: [:], force: true)
        guard invalid["event"]?["data"]["store_coverage"]["travel_time"]["status"] == .string("unsupported") else { throw SourceFailure.invalid }
        try execute("UPDATE CalendarItem SET travel_time=0 WHERE ROWID=1")
        let zero = await reader.enrich(observation, lookups: [:], force: true)
        guard zero["event"]?["data"]["travel_time"] == .number(0), zero["event"]?["data"]["store_coverage"]["travel_time"]["status"] == .string("complete") else { throw SourceFailure.invalid }
        for status in [2,4,3] {
            try execute("UPDATE Participant SET status=\(status) WHERE ROWID=1")
            let row = try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] event", external: "", calendar: "[ab-test] calendar", detached: false)
            guard row?["self_status"]?.number == Double(status) else { throw SourceFailure.invalid }
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
        try execute("DELETE FROM CalendarItem WHERE ROWID=1")
        guard try CalendarStoreSnapshot.read(path: path.path).event(identifier: "[ab-test] event", external: "", calendar: "[ab-test] calendar", detached: false) == nil else { throw SourceFailure.invalid }
        print("calendar parity checks passed")
    }
}
