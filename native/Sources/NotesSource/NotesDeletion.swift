import Foundation
import SQLite3
import SourceSupport

extension NotesStore {
    func deletionState(_ key: String, collection: String) throws -> String {
        guard ["notes.note", "notes.folder"].contains(collection) else { throw SourceFailure.invalid }
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if db != nil { sqlite3_close(db) }; throw SourceFailure.permissionMissing
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 1000)
        let entity = collection == "notes.note" ? "ICNote" : "ICFolder"
        let sql = """
        WITH RECURSIVE target AS (
          SELECT Z_PK,ZMARKEDFORDELETION,ZFOLDER FROM ZICCLOUDSYNCINGOBJECT
          WHERE ZIDENTIFIER=? AND Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='\(entity)')
        ), ancestors(pk,depth) AS (
          SELECT \(collection == "notes.note" ? "ZFOLDER" : "Z_PK"),0 FROM target
          UNION ALL SELECT f.ZPARENT,a.depth+1 FROM ZICCLOUDSYNCINGOBJECT f JOIN ancestors a ON f.Z_PK=a.pk WHERE a.depth<128
        )
        SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM Z_PRIMARYKEY WHERE Z_NAME='\(entity)') THEN 'unknown'
          WHEN NOT EXISTS(SELECT 1 FROM target) THEN 'gone'
          WHEN EXISTS(SELECT 1 FROM target WHERE COALESCE(ZMARKEDFORDELETION,0)<>0) THEN 'marked'
          WHEN EXISTS(SELECT 1 FROM ancestors a JOIN ZICCLOUDSYNCINGOBJECT f ON f.Z_PK=a.pk WHERE f.ZFOLDERTYPE=1 OR COALESCE(f.ZMARKEDFORDELETION,0)<>0) THEN 'trashed'
          ELSE 'active' END
        """
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        let bound = key.withCString { sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
        guard bound == SQLITE_OK, sqlite3_step(statement) == SQLITE_ROW, let value = sqlite3_column_text(statement, 0) else { throw SourceFailure.unavailable }
        return String(cString: value)
    }
}

extension NotesSource {
    static let deletionOperations = ["notes.delete", "notes.delete_folder"]
    func deletionCapabilities() -> [String] {
        let publicOperations = ["notes.delete"]
        guard let bytes = try? Data(contentsOf: deletionProofURL), let ledger = try? JSONDecoder().decode(JSON.self, from: bytes),
              ledger["version"].number == 1,
              ledger["os_build"].string == ProcessInfo.processInfo.operatingSystemVersionString,
              let generation = try? store.read().generation, ledger["generation"].string == generation else { return publicOperations }
        return publicOperations + Self.deletionOperations.filter { operation in
            guard !publicOperations.contains(operation) else { return false }
            let proof = ledger["operations"][operation]
            return proof["before_active"].bool && proof["command_completed"].bool && proof["notestore_verified"].bool && ["gone", "marked", "trashed"].contains(proof["after_state"].string)
        }
    }
    func requireDeletionProof(_ operation: String) throws {
        if Self.deletionOperations.contains(operation), !deletionCapabilities().contains(operation) { throw SourceFailure.capabilityUnproven }
    }

    #if LAB
    public func qualifyDeletion(_ operation: String, nativeKey: String, perform: () throws -> Void) async throws {
        guard Self.deletionOperations.contains(operation) else { throw SourceFailure.invalid }
        let collection = operation == "notes.delete" ? "notes.note" : "notes.folder"
        let snapshot = try store.read()
        guard let record = snapshot.observations.first(where: { $0["collection"].string == collection && $0["native_key"].string == nativeKey }),
              (collection == "notes.note" ? record["data"]["title"].string.hasPrefix("[ab-test") : record["data"]["title"].string.hasPrefix("Bridge Test [ab-test")),
              try store.deletionState(nativeKey, collection: collection) == "active" else { throw SourceFailure.invalid }
        var folder = collection == "notes.folder" ? record : snapshot.observations.first { $0["collection"].string == "notes.folder" && $0["native_key"] == record["data"]["folder_id"]["$ref"]["native_key"] } ?? .null
        var fixtureFolder = false
        for _ in 0..<128 where folder != .null {
            let title = folder["data"]["title"].string
            if title == "Bridge Test" || title.hasPrefix("Bridge Test [ab-test") { fixtureFolder = true; break }
            let parent = folder["data"]["parent_id"]["$ref"]["native_key"]
            folder = snapshot.observations.first { $0["collection"].string == "notes.folder" && $0["native_key"] == parent } ?? .null
        }
        guard fixtureFolder else { throw SourceFailure.invalid }
        var ledger = JSON.object(["version": .number(1), "os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString), "generation": .string(snapshot.generation), "operations": .object([:])])
        if let bytes = try? Data(contentsOf: deletionProofURL), let saved = try? JSONDecoder().decode(JSON.self, from: bytes), saved["os_build"] == ledger["os_build"], saved["generation"] == ledger["generation"] { ledger = saved }
        var operations = ledger["operations"].object
        operations.removeValue(forKey: operation)
        func save() throws {
            var value = ledger.object; value["operations"] = .object(operations)
            try JSONEncoder().encode(JSON.object(value)).write(to: deletionProofURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: deletionProofURL.path)
        }
        try save()
        try perform()
        for _ in 0..<100 {
            let state = try store.deletionState(nativeKey, collection: collection)
            if ["gone", "marked", "trashed"].contains(state) {
                operations[operation] = .object(["before_active": .bool(true), "command_completed": .bool(true), "notestore_verified": .bool(true), "after_state": .string(state)])
                try save(); return
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw SourceFailure.capabilityUnproven
    }
    public func deletionState(nativeKey: String, collection: String) throws -> String { try store.deletionState(nativeKey, collection: collection) }
    #endif
}
