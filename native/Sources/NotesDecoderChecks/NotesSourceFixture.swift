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
            if live {
                var sawNote = false
                for item in observations where item["kind"].string == "upsert" {
                    if item["collection"].string == "notes.note" { sawNote = true }
                    if item["collection"].string == "notes.folder", sawNote { throw Invalid.projection }
                }
            }
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
        try sql("CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER DEFAULT 1)")
        try sql("CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT,ZFILENAME TEXT,ZMERGEABLEDATA1 BLOB,ZALTTEXT TEXT,ZGENERATION TEXT,Z_OPT INTEGER DEFAULT 1)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Bridge Test Account')")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8,ZPARENT,ZFOLDERTYPE) VALUES(10,1,'root','Bridge Test',1,NULL,0),(11,1,'trash','Recently Deleted',1,NULL,1),(12,1,'nested','Nested',1,10,0)")
        try sql("CREATE TRIGGER object_revision AFTER UPDATE ON ZICCLOUDSYNCINGOBJECT WHEN NEW.Z_OPT=OLD.Z_OPT BEGIN UPDATE ZICCLOUDSYNCINGOBJECT SET Z_OPT=OLD.Z_OPT+1 WHERE Z_PK=NEW.Z_PK; END")
        try sql("CREATE TRIGGER body_revision AFTER UPDATE ON ZICNOTEDATA WHEN NEW.Z_OPT=OLD.Z_OPT BEGIN UPDATE ZICNOTEDATA SET Z_OPT=OLD.Z_OPT+1 WHERE Z_PK=NEW.Z_PK; END")
        let body = try Data(contentsOf: fixtures.appendingPathComponent("full.gz"))
        let table = try Data(contentsOf: fixtures.appendingPathComponent("full-table.gz"))
        try sql("INSERT INTO ZICNOTEDATA(Z_PK,ZDATA) VALUES(1,?)", blob: body)
        try sql("INSERT INTO ZICNOTEDATA(Z_PK,ZDATA) VALUES(2,?)", blob: Data(contentsOf: fixtures.appendingPathComponent("large-body.gz")))
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA,ZISPINNED,ZISPASSWORDPROTECTED) VALUES(20,2,'note','[ab-test] Full',12,1,1,0),(21,2,'locked','[ab-test] Locked',12,1,0,1),(22,2,'deleted','[ab-test] Deleted',11,1,0,0)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(24,2,'large','[ab-test] Large',12,2)")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNOTE1,ZMEDIA,ZTYPEUTI1) VALUES(30,4,'fixture-image',20,31,'public.data'),(32,4,'fixture-table',20,NULL,'com.apple.notes.table')")
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZFILENAME) VALUES(31,5,'media','fixture.bin')")
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZMERGEABLEDATA1=? WHERE Z_PK=32", blob: table)
        let media = root.appendingPathComponent("Accounts/account/Media/media/fixture.bin")
        try FileManager.default.createDirectory(at: media.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let original = Data("[ab-test] exact Notes attachment bytes 🧪".utf8)
        try original.write(to: media)
        for (pk, name) in [(3, "native-plain"), (4, "native-rich"), (5, "split-title"), (6, "title-attachment")] {
            try sql("INSERT INTO ZICNOTEDATA(Z_PK,ZDATA) VALUES(\(pk),?)", blob: Data(contentsOf: fixtures.appendingPathComponent(name + ".gz")))
            try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(\(40 + pk),2,'\(name)','[ab-test] Synthetic',12,\(pk))")
        }
        let traceURL = root.appendingPathComponent("capture-trace.jsonl")
        let source = try NotesSource(home: home, fixtureDatabase: database, trace: CaptureTrace(url: traceURL)), sink = Sink()
        guard !(await source.open()).capabilities.contains("notes.delete") else { throw Invalid.boundary }
        for operation in ["notes.delete", "notes.delete_folder"] {
            let action = JSON.object(["op": .string(operation), "native_key": .string("native-plain")])
            do { _ = try await source.prepare(.object(["action": action])); throw Invalid.boundary }
            catch SourceFailure.capabilityUnproven {}
            do { try source.validate(Prepared(nativeState: .null, fingerprint: "", action: action)); throw Invalid.boundary }
            catch SourceFailure.capabilityUnproven {}
        }
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        let note = await sink.record("note"), locked = await sink.record("locked"), deleted = await sink.record("deleted")
        let large = await sink.record("large")
        let plain = await sink.record("native-plain"), rich = await sink.record("native-rich")
        guard plain["data"]["editable"].string == "plain", plain["data"]["content_status"].string == "complete",
              rich["data"]["editable"].string == "none" else { throw Invalid.projection }
        for candidate in [plain, rich] {
            let action = JSON.object(["op": .string("notes.update"), "native_key": candidate["native_key"], "mode": .string("append"), "plain_text": .string("[ab-test] append")])
            do {
                _ = try await source.prepare(.object(["action": action, "expected_state": candidate["data"]]))
                guard candidate == plain else { throw Invalid.boundary }
            } catch SourceFailure.unsupported {
                guard candidate == rich else { throw Invalid.boundary }
            }
        }
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
            for (id, titles) in [("native-plain", 0), ("split-title", 1), ("title-attachment", 1)] {
                let truth = response["records"].array.first { $0["id"].string == id } ?? .null
                let projection = await sink.record(id)
                guard truth["content_status"].string == "complete", truth["structure_counts"]["titles"] != .null,
                      truth["structure_counts"]["titles"].number == Double(titles),
                      projection["data"]["blocks"].array.filter({ $0["type"].string == "title" }).count == titles else { throw Invalid.projection }
            }
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
            let action = JSON.object(["op": .string("notes.update"), "native_key": protected["native_key"]])
            do { _ = try await source.prepare(.object(["action": action, "expected_state": protected["data"]])); throw Invalid.boundary }
            catch SourceFailure.unsupported {}
        }
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER) VALUES(60,2,'delete-proof','[ab-test] Deletion proof',10)")
        guard try source.deletionState(nativeKey: "delete-proof", collection: "notes.note") == "active",
              try source.deletionState(nativeKey: "deleted", collection: "notes.note") == "trashed",
              try source.deletionState(nativeKey: "absent", collection: "notes.note") == "gone" else { throw Invalid.boundary }
        do { try await source.qualifyDeletion("notes.delete", nativeKey: "delete-proof", perform: {}); throw Invalid.boundary }
        catch SourceFailure.capabilityUnproven {}
        guard !(await source.open()).capabilities.contains("notes.delete") else { throw Invalid.boundary }
        try await source.qualifyDeletion("notes.delete", nativeKey: "delete-proof") { try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZMARKEDFORDELETION=1 WHERE Z_PK=60") }
        guard try source.deletionState(nativeKey: "delete-proof", collection: "notes.note") == "marked",
              (await source.open()).capabilities.contains("notes.delete") else { throw Invalid.boundary }
        for protected in [locked, deleted] {
            let action = JSON.object(["op": .string("notes.delete"), "native_key": protected["native_key"]])
            do { _ = try await source.prepare(.object(["action": action, "expected_state": protected["data"]])); throw Invalid.boundary }
            catch SourceFailure.unsupported {}
        }
        try sql("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8) VALUES(13,1,'delete-folder-proof','Bridge Test [ab-test] Deletion proof',1)")
        guard try source.deletionState(nativeKey: "delete-folder-proof", collection: "notes.folder") == "active" else { throw Invalid.boundary }
        try await source.qualifyDeletion("notes.delete_folder", nativeKey: "delete-folder-proof") { try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZMARKEDFORDELETION=1 WHERE Z_PK=13") }
        guard try source.deletionState(nativeKey: "delete-folder-proof", collection: "notes.folder") == "marked" else { throw Invalid.boundary }
        let proofURL = home.appendingPathComponent("notes-deletion-proofs.json")
        var proof = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: proofURL)).object
        let validProof = proof
        proof["os_build"] = .string("different-build")
        try JSONEncoder().encode(JSON.object(proof)).write(to: proofURL)
        guard !(await source.open()).capabilities.contains("notes.delete") else { throw Invalid.boundary }
        proof = validProof; proof["generation"] = .string("different-store")
        try JSONEncoder().encode(JSON.object(proof)).write(to: proofURL)
        guard !(await source.open()).capabilities.contains("notes.delete") else { throw Invalid.boundary }
        if let executable = ProcessInfo.processInfo.environment["APPLE_BRIDGE_NOTES_TRUTH"], !executable.isEmpty {
            for (ids, expected) in [(["ICNote/p20"], false), (["ICNote/p22","ICNote/p60","ICNote/p9999","ICFolder/p13"], true), (["ICFolder/p10"], false)] {
                let request = root.appendingPathComponent("deletion-request.json"), report = root.appendingPathComponent("deletion-report.json")
                try JSONSerialization.data(withJSONObject: ["source":"notes", "snapshot_path":database.path, "verify_deleted_ids":ids.map { "x-coredata://ab-test-notes-store/\($0)" }]).write(to: request)
                let process = Process(); process.executableURL = URL(fileURLWithPath: executable)
                process.arguments = ["observe", "--request-file", request.path, "--report", report.path]
                process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
                try process.run(); process.waitUntilExit()
                let result = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: report))
                guard process.terminationStatus == 0, result["status"].string == "verified", result["deletion_verified"].bool == expected else { throw Invalid.boundary }
            }
        }
        print("Notes deletion: no-op command cannot qualify; marked/trash/gone states and OS-build invalidation verified")
        do { _ = try source.readBlob("notes-blob:" + String(repeating: "0", count: 64), offset: 0, length: 1); throw Invalid.boundary }
        catch SourceFailure.unavailable {}
        let hold = home.appendingPathComponent("lab-notes-hold-hydration")
        try Data().write(to: hold)
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Incremental 1' WHERE Z_PK=20")
        try await source.reconcile(into: sink)
        let first = await sink.record("note")
        guard first["data"]["title"].string == "[ab-test] Incremental 1", first["data"]["body_status"].string == "pending", first["data"]["content_status"].string == "partial",
              !first["data"]["plain_text"].string.isEmpty, first["data"]["blocks"] == .null,
              source.lastChangedNotes == 1, source.bootstrapScans == 1 else { throw Invalid.projection }
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='[ab-test] Incremental 2' WHERE Z_PK=20")
        try await source.reconcile(into: sink)
        guard source.lastChangedNotes == 1 else { throw Invalid.projection }
        try FileManager.default.removeItem(at: hold)
        try await source.waitForHydration()
        let final = await sink.record("note")
        guard final["data"]["title"].string == "[ab-test] Incremental 2", final["data"]["blocks"].array.first?["type"].string == "title" else { throw Invalid.projection }
        try sql("UPDATE ZICNOTEDATA SET ZDATA=? WHERE Z_PK=2", blob: body)
        try await source.reconcile(into: sink)
        guard source.lastChangedNotes == 1, (await sink.record("large"))["data"]["plain_text"] == note["data"]["plain_text"] else { throw Invalid.projection }
        try await source.waitForHydration()
        try sql("DELETE FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=24")
        try await source.reconcile(into: sink)
        guard (await sink.record("large"))["kind"].string == "absent" else { throw Invalid.projection }
        try sql("ALTER TABLE ZICCLOUDSYNCINGOBJECT ADD COLUMN ZSCHEMAFIXTURE INTEGER")
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        guard source.bootstrapScans == 2 else { throw Invalid.projection }
        let events = try String(contentsOf: traceURL, encoding: .utf8).split(separator: "\n").map { try JSONDecoder().decode(JSON.self, from: Data($0.utf8)) }
        let started = events.first { $0["stage"].string == "notes_hydration_started" }?["uptime_ns"].number ?? 0
        let committed = events.first { $0["stage"].string == "notes_capture_committed" }?["uptime_ns"].number ?? 0
        guard committed > 0, started > committed else { throw Invalid.projection }
        print("Notes metadata commits before hydration; single-row/body-only changes, stale hydration, hard deletion and schema checkpoint fallback passed")
        let downloaded = Data("[ab-test] replacement Native media bytes".utf8)
        try downloaded.write(to: media)
        try await source.reconcile(into: sink, assetPaths: [media.standardizedFileURL.path])
        try await source.waitForHydration()
        let downloadedBlob = (await sink.record("note"))["data"]["attachments"].array.first { $0["id"].string == "fixture-image" }?["blob"] ?? .null
        guard source.lastChangedNotes == 0, downloadedBlob["digest"].string == SHA256.hash(data: downloaded).map({ String(format: "%02x", $0) }).joined() else { throw Invalid.blob }
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZNOTE1=43 WHERE Z_PK=30")
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        let detachedBlob = (await sink.record("note"))["data"]["attachments"].array.first { $0["id"].string == "fixture-image" }?["blob"] ?? .null
        guard source.lastChangedNotes == 2, detachedBlob == .null else { throw Invalid.boundary }
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZNOTE1=20 WHERE Z_PK=30")
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        let restoredBlob = (await sink.record("note"))["data"]["attachments"].array.first { $0["id"].string == "fixture-image" }?["blob"] ?? .null
        guard source.lastChangedNotes == 2, restoredBlob["digest"] == downloadedBlob["digest"] else { throw Invalid.boundary }
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFILENAME='../escape.bin' WHERE Z_PK=31")
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        let unsafe = (await sink.record("note"))["data"]["attachments"].array.first { $0["id"].string == "fixture-image" } ?? .null
        guard unsafe["state"].string == "unavailable", unsafe["blob"] == .null else { throw Invalid.boundary }
        try sql("UPDATE ZICCLOUDSYNCINGOBJECT SET ZPARENT=11 WHERE Z_PK=12")
        try await source.reconcile(into: sink)
        try await source.waitForHydration()
        guard (await sink.record("note"))["data"]["read_only"].bool, (await sink.record("locked"))["data"]["read_only"].bool else { throw Invalid.boundary }
        let moved = root.appendingPathComponent("unavailable.sqlite")
        try FileManager.default.moveItem(at: database, to: moved)
        defer { try? FileManager.default.moveItem(at: moved, to: database) }
        do { _ = try source.readBlob(handle, offset: 0, length: 1); throw Invalid.boundary }
        catch SourceFailure.permissionMissing {}
        print("Notes SQLite projection, locked/foreign/path boundaries and exact private blob bytes passed")
    }
}
#endif
