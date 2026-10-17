import Foundation
import CryptoKit
import SourceSupport

@MainActor
public final class NotesSource: Source {
    public nonisolated let name = "notes"
    public static let operations = ["notes.create", "notes.update", "notes.delete", "notes.create_folder"]
    private let store: NotesStore
    private let baselineURL: URL
    private var previous: [String: JSON] = [:]
    private var generation = ""
    private var scanning = false
    public init(home: URL) throws {
        store = NotesStore(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path)
        baselineURL = home.appendingPathComponent("notes-identities.json")
        if let bytes = try? Data(contentsOf: baselineURL), let saved = try? JSONDecoder().decode(JSON.self, from: bytes) {
            generation = saved["generation"].string
            for item in saved["identities"].array { previous[item["collection"].string + "/" + item["native_key"].string] = item }
        }
    }
    public func open() async -> SourceReadiness {
        let readable = (try? store.read()) != nil
        return SourceReadiness(state: readable ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("rich content read as plain text; attachments metadata only")])]), capabilities: readable ? Self.operations : [], reason: readable ? "" : "notes_database_access_required")
    }
    public func reconcile(into sink: any ObservationSink) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        scanning = true
        defer { scanning = false }
        let started = timestamp()
        let current = try store.read()
        if generation != current.generation { previous = [:]; generation = current.generation }
        let records = Dictionary(uniqueKeysWithValues: current.observations.map { ($0["collection"].string + "/" + $0["native_key"].string, $0) })
        var changes = current.observations.filter { previous[$0["collection"].string + "/" + $0["native_key"].string]?["data"] != $0["data"] }
        for (key, item) in previous where records[key] == nil {
            changes.append(.object(["kind": .string("absent"), "collection": item["collection"], "native_key": item["native_key"], "observed_at": .string(timestamp()), "revalidated": .bool(true)]))
        }
        try await sink.emit(source: name, generation: generation, observations: changes, scanID: UUID().uuidString, scanStartedAt: started, scope: ["notes.folder", "notes.note"], completionEvidence: "complete", blobHandles: [:], live: true)
        previous = records
        let identities = records.values.map { JSON.object(["collection": $0["collection"], "native_key": $0["native_key"]]) }
        try JSONEncoder().encode(JSON.object(["generation": .string(generation), "identities": .array(identities)])).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = NotesWatcher(path: URL(fileURLWithPath: store.path).deletingLastPathComponent().path)
        defer { watcher.close() }
        for await _ in watcher.changes {
            try Task.checkCancellation()
            try await reconcile(into: sink)
        }
    }
    private func find(_ key: String, collection: String = "notes.note") throws -> JSON {
        guard let note = try store.read().observations.first(where: { $0["native_key"].string == key && $0["collection"].string == collection }) else { throw SourceFailure.conflict }
        return note
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        guard Self.operations.contains(action["op"].string) else { throw SourceFailure.unsupported }
        var before = JSON.null
        if !action["native_key"].string.isEmpty {
            let note = try find(action["native_key"].string)
            before = note["data"]
            if action["op"].string == "notes.update", before["editable"].string != "plain" { throw SourceFailure.unsupported }
            var expected = command["expected_state"].object
            var actual = before.object
            expected.removeValue(forKey: "folder_id"); actual.removeValue(forKey: "folder_id")
            guard expected == actual else { throw SourceFailure.conflict }
        }
        if action["op"].string == "notes.create" { _ = try find(action["folder_key"].string, collection: "notes.folder") }
        let bytes = try JSONEncoder().encode(action)
        return Prepared(nativeState: before, fingerprint: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(), action: action)
    }
    public func validate(_ prepared: Prepared) throws {
        if !prepared.action["native_key"].string.isEmpty {
            guard try find(prepared.action["native_key"].string)["data"] == prepared.nativeState else { throw SourceFailure.conflict }
        }
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        try validate(prepared)
        let action = prepared.action, args = action["note_args"]
        let op = action["op"].string
        var scriptID = ""
        if op == "notes.create_folder" {
            scriptID = try NotesScript.run("return id of (make new folder with properties {name:" + NotesScript.quote(args["title"].string) + "})")
        } else if op == "notes.create" {
            let folder = try find(action["folder_key"].string, collection: "notes.folder")
            let content = args["title"].string + "\n" + args["plain_text"].string
            scriptID = try NotesScript.run("return id of (make new note at folder id " + NotesScript.quote(folder["scoped_alias"].string) + " with properties {body:" + NotesScript.quote(NotesScript.html(content)) + "})")
        } else {
            let note = try find(action["native_key"].string)
            scriptID = note["scoped_alias"].string
            if op == "notes.delete" { _ = try NotesScript.run("delete note id " + NotesScript.quote(scriptID)) }
            else {
                let content = args["mode"].string == "append" ? note["data"]["plain_text"].string + args["plain_text"].string : args["plain_text"].string
                _ = try NotesScript.run("set body of note id " + NotesScript.quote(scriptID) + " to " + NotesScript.quote(NotesScript.html(content)))
            }
        }
        for _ in 0..<100 {
            let snapshot = try store.read()
            let observed = snapshot.observations.first { $0["scoped_alias"].string == scriptID }
            if op == "notes.delete", observed == nil { return .object(["native_key": action["native_key"], "observed": .bool(true), "absent": .bool(true)]) }
            if let observed, op != "notes.delete" {
                if op != "notes.update" || observed["data"]["plain_text"] != prepared.nativeState["plain_text"] {
                    return .object(["native_key": observed["native_key"], "observed": .bool(true)])
                }
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw SourceFailure.unavailable
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
