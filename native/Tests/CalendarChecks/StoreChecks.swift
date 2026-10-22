import Foundation
import SQLite3
import CalendarSource
import AgendaGate
import SourceSupport

struct CalendarStoreChecks {
    @AgendaActor
    static func run() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let path = root.appendingPathComponent("Calendar.sqlitedb")
        try Data("test".utf8).write(to: root.appendingPathComponent("fixture.txt"))
        var database: OpaquePointer?
        guard sqlite3_open(path.path, &database) == SQLITE_OK else { throw CocoaError(.fileWriteUnknown) }
        defer { sqlite3_close(database) }
        let schema = "CREATE TABLE Calendar(ROWID INTEGER PRIMARY KEY,UUID TEXT); CREATE TABLE Location(ROWID INTEGER PRIMARY KEY,title TEXT,address TEXT,latitude INTEGER,longitude INTEGER,radius INTEGER); CREATE TABLE Participant(ROWID INTEGER PRIMARY KEY,status INTEGER); CREATE TABLE Conference(owner_id INTEGER,url TEXT); CREATE TABLE CalendarItem(ROWID INTEGER PRIMARY KEY,unique_identifier TEXT,UUID TEXT,external_id TEXT,calendar_id INTEGER,location_id INTEGER,self_attendee_id INTEGER,travel_time INTEGER,travel_advisory_behavior INTEGER,invitation_status INTEGER,privacy_level INTEGER,priority INTEGER,conference_url TEXT); CREATE TABLE Attachment(owner_id INTEGER,file_id INTEGER); CREATE TABLE AttachmentFile(ROWID INTEGER PRIMARY KEY,UUID TEXT,filename TEXT,local_path TEXT,file_size INTEGER,format TEXT); INSERT INTO Calendar VALUES(1,'calendar-a'),(2,'calendar-b'); INSERT INTO Location VALUES(1,'Bridge Test Station','Example Street',15000000,25000000,100); INSERT INTO Participant VALUES(1,2); INSERT INTO CalendarItem VALUES(1,'event-a','uuid-a','shared',1,1,1,1200,1,2,0,5,'https://example.com/meeting'),(2,'event-b','uuid-b','shared',2,NULL,NULL,0,0,0,0,0,NULL); INSERT INTO AttachmentFile VALUES(1,'file-a','fixture.txt','fixture.txt',4,'text/plain'); INSERT INTO Attachment VALUES(1,1);"
        guard sqlite3_exec(database, schema, nil, nil, nil) == SQLITE_OK else { throw CocoaError(.fileWriteUnknown) }
        let snapshot = try CalendarStoreSnapshot.read(path: path.path)
        let first = snapshot.event(identifier: "event-a", external: "", calendar: "calendar-a", detached: false)
        let reader = CalendarStoreEnrichment(root: root)
        let observations: [String: JSON] = ["event": .object(["collection": .string("calendar.event"), "native_key": .string("event-a"), "data": .object(["calendar_id": .object(["$ref": .object(["native_key": .string("calendar-a")])])])])]
        let enriched = await reader.enrich(observations, lookups: [:], force: true)
        let attachment = enriched["event"]?["data"]["attachments"].array.first ?? .null
        let digest = attachment["blob"]["digest"].string
        guard let handle = reader.handles[digest] else { throw CocoaError(.coderReadCorrupt) }
        let bytes = try reader.readBlob(handle, offset: 0, length: 128)
        let originalDate = try FileManager.default.attributesOfItem(atPath: root.appendingPathComponent("fixture.txt").path)[.modificationDate]!
        let outside = root.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".txt")
        try Data("test".utf8).write(to: outside)
        defer { try? FileManager.default.removeItem(at: outside) }
        try FileManager.default.setAttributes([.modificationDate: originalDate], ofItemAtPath: outside.path)
        try FileManager.default.removeItem(at: root.appendingPathComponent("fixture.txt"))
        try FileManager.default.createSymbolicLink(atPath: root.appendingPathComponent("fixture.txt").path, withDestinationPath: outside.path)
        var escaped = false
        do { _ = try reader.readBlob(handle, offset: 0, length: 128) } catch { escaped = true }
        let checks = [
            "store_blob_digest_and_bytes": digest == "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" && Data(base64Encoded: bytes["bytes"].string) == Data("test".utf8),
            "store_blob_scope_cannot_escape_calendar_root": escaped,
            "store_unverified_units_stay_hidden": enriched["event"]?["data"]["travel_time"] == .null && enriched["event"]?["data"]["structured_location"]["latitude"] == .null,
            "store_unique_identifier_join": first?["travel_time"]?.number == 1200,
            "store_uuid_join": snapshot.event(identifier: "uuid-a", external: "", calendar: "calendar-a", detached: false) != nil,
            "store_external_identity_is_calendar_scoped": snapshot.event(identifier: "unknown", external: "shared", calendar: "calendar-b", detached: false)?["item_id"]?.number == 2,
            "store_unknown_calendar_rejects": snapshot.event(identifier: "event-a", external: "shared", calendar: "calendar-c", detached: false) == nil,
            "store_detached_identity_is_not_guessed": snapshot.event(identifier: "event-a", external: "shared", calendar: "calendar-a", detached: true) == nil,
            "store_fields_decoded_without_unit_guessing": first?["latitude"]?.number == 15000000 && first?["self_status"]?.number == 2 && first?["conference_url"]?.string == "https://example.com/meeting"
        ]
        guard checks.values.allSatisfy({ $0 }) else { throw CocoaError(.coderReadCorrupt) }
        print(String(decoding: try JSONSerialization.data(withJSONObject: checks, options: [.sortedKeys]), as: UTF8.self))
    }
}
