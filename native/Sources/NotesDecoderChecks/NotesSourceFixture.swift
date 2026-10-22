#if LAB
import Foundation
import SQLite3
import CryptoKit
import NotesSource
import SourceSupport
import Darwin

enum NotesSourceFixture {
    private enum Invalid: Error { case sqlite, projection, blob, boundary }
    private actor Sink: ObservationSink {
        var records: [String: JSON] = [:]
        var handles: [String: String] = [:]
        func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
        func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
            for item in observations { records[item["native_key"].string] = item }
            handles.merge(blobHandles) { _, new in new }
        }
        func record(_ key: String) -> JSON { records[key] ?? .null }
        func handle(_ digest: String) -> String? { handles[digest] }
    }
    @MainActor
    static func run(_ fixtures: URL) async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-notes-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("NoteStore.sqlite"), home = root.appendingPathComponent("helper")
        var db: OpaquePointer?
        guard sqlite3_open(database.path, &db) == SQLITE_OK else { throw Invalid.sqlite }
        defer { sqlite3_close(db) }
        @MainActor func sql(_ value: String, blob: Data? = nil) throws {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(db, value, -1, &statement, nil) == SQLITE_OK else { throw Invalid.sqlite }
            defer { sqlite3_finalize(statement) }
            if let blob {
                let data = blob as NSData
                let code = sqlite3_bind_blob(statement, 1, data.bytes, Int32(data.length), unsafeBitCast(-1, to: sqlite3_destructor_type.self))
                withExtendedLifetime(data) {}
                guard code == SQLITE_OK else { throw Invalid.sqlite }
            }
            guard sqlite3_step(statement) == SQLITE_DONE else { throw Invalid.sqlite }
        }
        try sql("CREATE TABLE Z_METADATA(Z_UUID TEXT)")
        try sql("INSERT INTO Z_METADATA VALUES('ab-test-notes-store')")
        try sql("CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT)")
        try sql("INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount'),(4,'ICAttachment'),(5,'ICMedia')")
        try sql("CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB)")
        try sql("CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT,ZFILENAME TEXT,ZMERGEABLEDATA1 BLOB,ZALTTEXT TEXT,ZGENERATION TEXT)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Bridge Test Account')")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8,ZPARENT,ZFOLDERTYPE) VALUES(10,1,'root','Bridge Test',1,NULL,0),(11,1,'trash','Recently Deleted',1,NULL,1),(12,1,'nested','Nested',1,10,0)")
        let body = try Data(contentsOf: fixtures.appendingPathComponent("full.gz"))
        let table = try Data(contentsOf: fixtures.appendingPathComponent("full-table.gz"))
        try sql("INSERT INTO ZICNOTEDATA VALUES(1,?)", blob: body)
        try sql("INSERT INTO ZICNOTEDATA VALUES(2,?)", blob: Data(contentsOf: fixtures.appendingPathComponent("large-body.gz")))
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA,ZISPINNED,ZISPASSWORDPROTECTED) VALUES(20,2,'note','[ab-test] Full',12,1,1,0),(21,2,'locked','[ab-test] Locked',12,1,0,1),(22,2,'deleted','[ab-test] Deleted',11,1,0,0)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(24,2,'large','[ab-test] Large',12,2)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNOTE1,ZMEDIA,ZTYPEUTI1) VALUES(30,4,'fixture-image',20,31,'public.data'),(32,4,'fixture-table',20,NULL,'com.apple.notes.table')")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZFILENAME) VALUES(31,5,'media','fixture.bin')")
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZMERGEABLEDATA1=? WHERE Z_PK=32", blob: table)
        let media = root.appendingPathComponent("Accounts/account/Media/media/fixture.bin")
        try FileManager.default.createDirectory(at: media.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let original = Data("[ab-test] exact Notes attachment bytes 🧪".utf8)
        try original.write(to: media)
        let source = try NotesSource(home: home, fixtureDatabase: database), sink = Sink()
        try await source.reconcile(into: sink)
        let note = await sink.record("note"), locked = await sink.record("locked"), deleted = await sink.record("deleted")
        let large = await sink.record("large")
        let bodyRef = large["data"]["body_blob"]
        guard large["data"]["body_status"].string == "external", large["data"]["editable"].string == "none",
              large["data"]["plain_text"] == .null, large["data"]["preview"].string == String(repeating: "x", count: 160),
              bodyRef["size"].number > 262144, let bodyHandle = await sink.handle(bodyRef["digest"].string) else { throw Invalid.projection }
        var payload = Data()
        while true {
            let chunk = try source.readBlob(bodyHandle, offset: payload.count, length: 131072)
            guard let bytes = Data(base64Encoded: chunk["bytes"].string), !bytes.isEmpty else { throw Invalid.blob }
            payload.append(bytes)
            if chunk["eof"].bool { break }
        }
        let decoded = try JSONDecoder().decode(JSON.self, from: payload)
        guard decoded["plain_text"].string == "[ab-test] Large\n" + String(repeating: "x", count: 250000) + "\n",
              SHA256.hash(data: payload).map({ String(format: "%02x", $0) }).joined() == bodyRef["digest"].string,
              try JSONEncoder().encode(large).count < 224000 else { throw Invalid.blob }
        print("Large Notes body: exact 250000-byte text via verified blob; metadata below the unchanged frame limit")
        guard note["data"]["pinned"].bool, note["data"]["blocks"].array.first?["type"].string == "title",
              locked["data"]["content_status"].string == "locked", locked["data"]["plain_text"].string.isEmpty,
              locked["data"]["attachments"].array.isEmpty, locked["data"]["blocks"] == .null,
              deleted["data"]["read_only"].bool, deleted["data"]["editable"].string == "none",
              (await sink.record("nested"))["data"]["account_name"].string == "Bridge Test Account" else { throw Invalid.projection }
        if let executable = ProcessInfo.processInfo.environment["APPLE_BRIDGE_NOTES_TRUTH"], !executable.isEmpty {
            let request = root.appendingPathComponent("truth-request.json"), report = root.appendingPathComponent("truth-report.json")
            try JSONSerialization.data(withJSONObject: ["source": "notes", "sqlite": true, "snapshot_path": database.path]).write(to: request)
            let process = Process()
            process.executableURL = URL(fileURLWithPath: executable)
            process.arguments = ["observe", "--request-file", request.path, "--report", report.path]
            process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
            try process.run(); process.waitUntilExit()
            let response = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: report))
            let independent = response["records"].array.first { $0["id"].string == "note" } ?? .null
            let expected: [String: Int] = ["titles": 1, "headings": 2, "monostyled": 1, "list_items": 3, "checklists": 2, "checked": 1, "quotes": 1, "highlighted_paragraphs": 1, "links": 1, "attachments": 2, "tables": 1, "table_cells": 4]
            guard process.terminationStatus == 0, response["status"].string == "verified", independent["plain_text"] == note["data"]["plain_text"],
                  expected.allSatisfy({ independent["structure_counts"][$0.key].number == Double($0.value) }) else { throw Invalid.projection }
            print("Independent AppleTruth: 13 block paragraphs, 2 checklist states, 4 table cells")
        }
        let attachment = note["data"]["attachments"].array.first { $0["id"].string == "fixture-image" } ?? .null
        let digest = SHA256.hash(data: original).map { String(format: "%02x", $0) }.joined()
        guard attachment["blob"]["digest"].string == digest, let handle = await sink.handle(digest) else { throw Invalid.blob }
        let chunk = try source.readBlob(handle, offset: 0, length: 131072)
        guard Data(base64Encoded: chunk["bytes"].string) == original, chunk["eof"].bool else { throw Invalid.blob }
        let staged = home.appendingPathComponent("notes-blobs/" + digest)
        let attrs = try FileManager.default.attributesOfItem(atPath: staged.path)
        guard (attrs[.posixPermissions] as? NSNumber)?.intValue == 0o600,
              (attrs[.ownerAccountID] as? NSNumber)?.uint32Value == geteuid() else { throw Invalid.boundary }
        guard deleted["data"]["attachments"].array.allSatisfy({ $0["blob"] == .null }) else { throw Invalid.boundary }
        for protected in [locked, deleted] {
            let action = JSON.object(["op": .string("notes.delete"), "native_key": protected["native_key"]])
            do { _ = try await source.prepare(.object(["action": action, "expected_state": protected["data"]])); throw Invalid.boundary }
            catch SourceFailure.unsupported {}
        }
        do { _ = try source.readBlob("notes-blob:" + String(repeating: "0", count: 64), offset: 0, length: 1); throw Invalid.boundary }
        catch SourceFailure.unavailable {}
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFILENAME='../escape.bin' WHERE Z_PK=31")
        try await source.reconcile(into: sink)
        let unsafe = (await sink.record("note"))["data"]["attachments"].array.first { $0["id"].string == "fixture-image" } ?? .null
        guard unsafe["state"].string == "unavailable", unsafe["blob"] == .null else { throw Invalid.boundary }
        let moved = root.appendingPathComponent("unavailable.sqlite")
        try FileManager.default.moveItem(at: database, to: moved)
        defer { try? FileManager.default.moveItem(at: moved, to: database) }
        do { _ = try source.readBlob(handle, offset: 0, length: 1); throw Invalid.boundary }
        catch SourceFailure.permissionMissing {}
        print("Notes SQLite projection, locked/foreign/path boundaries and exact private blob bytes passed")
    }
}
#endif
