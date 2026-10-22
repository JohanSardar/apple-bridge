import Foundation
import CryptoKit
import SourceSupport
import MediaDerivatives

@MainActor
public final class NotesSource: Source {
    public nonisolated let name = "notes"
    public static let operations = ["notes.create", "notes.update", "notes.delete", "notes.create_folder"]
    private let store: NotesStore
    private let baselineURL: URL
    private let assets: NotesAssets
    private var previous: [String: JSON] = [:]
    private var generation = ""
    private var scanning = false
    public convenience init(home: URL) throws {
        try self.init(home: home, database: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite"))
    }
    #if LAB
    public convenience init(home: URL, fixtureDatabase: URL) throws {
        try self.init(home: home, database: fixtureDatabase)
    }
    #endif
    private init(home: URL, database: URL) throws {
        assets = try NotesAssets(home: home)
        store = NotesStore(path: database.path)
        baselineURL = home.appendingPathComponent("notes-identities.json")
        if let bytes = try? Data(contentsOf: baselineURL), let saved = try? JSONDecoder().decode(JSON.self, from: bytes) {
            generation = saved["generation"].string
            for item in saved["identities"].array { previous[item["collection"].string + "/" + item["native_key"].string] = item }
        }
    }
    public func open() async -> SourceReadiness {
        let readable = (try? store.read()) != nil
        return SourceReadiness(state: readable ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("unknown protobuf fields and unavailable attachment bytes are explicit; rich edits unproven")])]), capabilities: readable ? Self.operations : [], reason: readable ? "" : "notes_database_access_required")
    }
    public func reconcile(into sink: any ObservationSink) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        scanning = true
        defer { scanning = false }
        let started = timestamp()
        let current = try store.read()
        if generation != current.generation { previous = [:]; generation = current.generation }
        var observations: [JSON] = [], references: [JSON] = []
        for observation in current.observations {
            let key = observation["collection"].string + "/" + observation["native_key"].string
            var value = observation.object, data = observation["data"].object
            if let old = previous[key], !data["native_fingerprint", default: .null].string.isEmpty,
               old["data"]["native_fingerprint"] == data["native_fingerprint"],
               !old["data"]["attachments"].array.contains(where: { $0["reason"].string == "attachment_staging_unavailable" || $0["variant_status"].string == "unavailable" }),
               old["data"]["body_blob"]["state"].string != "missing",
               old["data"]["raw_body"]["state"].string != "missing" {
                data = old["data"].object
            } else if observation["collection"].string == "notes.note", data["locked"]?.bool != true {
                var attachments: [JSON] = []
                for metadata in data["attachments", default: .array([])].array {
                    var item = metadata.object
                    if let attachment = current.attachments[observation["native_key"].string + ":" + metadata["id"].string], let file = attachment.file {
                        do {
                            let original = try await assets.stage(file)
                            item["blob"] = original; item["state"] = .string("present")
                            if let local = assets.file(original["digest"].string), MediaDerivatives.supports(file, uti: attachment.uti) {
                                do {
                                    var variants: [String: JSON] = [:]
                                    let result = try await MediaDerivatives.make(local, digest: original["digest"].string)
                                    for (name, variant) in result.derivatives {
                                        var reference = try await assets.stage(MediaDerivatives.url(variant)).object
                                        reference["variant"] = .string(name); reference["mime"] = .string(variant.mime)
                                        if let width = variant.width { reference["width"] = .number(Double(width)) }
                                        if let height = variant.height { reference["height"] = .number(Double(height)) }
                                        variants[name] = .object(reference)
                                    }
                                    item["derivatives"] = .object(variants)
                                    item["variant_status"] = .string(result.unavailable?.isEmpty == false ? "partial" : "complete")
                                } catch {
                                    try Task.checkCancellation()
                                    item["variant_status"] = .string("unavailable")
                                }
                            }
                        } catch {
                            try Task.checkCancellation()
                            item["state"] = .string("unavailable"); item["reason"] = .string("attachment_staging_unavailable")
                        }
                    } else if let payload = current.attachments[observation["native_key"].string + ":" + metadata["id"].string]?.payload {
                        do { item["blob"] = try await assets.stage(payload); item["state"] = .string("present") }
                        catch { item["state"] = .string("unavailable"); item["reason"] = .string("attachment_staging_unavailable") }
                    }
                    attachments.append(.object(item))
                }
                data["attachments"] = .array(attachments)
                if let bytes = current.rawBodies[observation["native_key"].string] {
                    do { data["raw_body"] = try await assets.stage(bytes) }
                    catch {
                        data["raw_body"] = .object(["digest": data["raw_body_digest"] ?? .null, "size": .number(Double(bytes.count)), "state": .string("missing"), "reason": .string("native_staging_unavailable")])
                    }
                }
            }
            if observation["collection"].string == "notes.note" { data = try await NotesBodyProjection.fit(data, assets: assets) }
            references.append(contentsOf: NotesBodyProjection.references(data))
            value["data"] = .object(data)
            observations.append(.object(value))
        }
        let records = Dictionary(uniqueKeysWithValues: observations.map { ($0["collection"].string + "/" + $0["native_key"].string, $0) })
        var changes = observations.filter { previous[$0["collection"].string + "/" + $0["native_key"].string]?["data"] != $0["data"] }
        for (key, item) in previous where records[key] == nil {
            changes.append(.object(["kind": .string("absent"), "collection": item["collection"], "native_key": item["native_key"], "observed_at": .string(timestamp()), "revalidated": .bool(true)]))
        }
        try await sink.emit(source: name, generation: generation, observations: changes, scanID: UUID().uuidString, scanStartedAt: started, scope: ["notes.folder", "notes.note"], completionEvidence: "complete", blobHandles: assets.handles(references), live: true)
        previous = records
        try assets.prune(references)
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
    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard let access = try? FileHandle(forReadingFrom: URL(fileURLWithPath: store.path)) else { throw SourceFailure.permissionMissing }
        try access.close()
        return try assets.read(handle, offset: offset, length: length)
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
            guard !before["locked"].bool, !before["read_only"].bool, !before["recently_deleted"].bool else { throw SourceFailure.unsupported }
            guard command["expected_state"]["native_fingerprint"] == before["native_fingerprint"] else { throw SourceFailure.conflict }
            let fingerprint = before["native_fingerprint"], text = before["plain_text"]
            before = .object(["native_fingerprint": fingerprint])
            if action["op"].string == "notes.update" { before = .object(["native_fingerprint": fingerprint, "plain_text": text]) }
        }
        if action["op"].string == "notes.create" {
            let folder = try find(action["folder_key"].string, collection: "notes.folder")
            guard !folder["data"]["read_only"].bool else { throw SourceFailure.unsupported }
        }
        let bytes = try JSONEncoder().encode(action)
        return Prepared(nativeState: before, fingerprint: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(), action: action)
    }
    public func validate(_ prepared: Prepared) throws {
        if !prepared.action["native_key"].string.isEmpty {
            guard try find(prepared.action["native_key"].string)["data"]["native_fingerprint"] == prepared.nativeState["native_fingerprint"] else { throw SourceFailure.conflict }
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
            if op == "notes.delete" {
                let folderKey = note["data"]["folder_id"]["$ref"]["native_key"].string
                let folder = try find(folderKey, collection: "notes.folder")
                _ = try NotesScript.run("delete note id " + NotesScript.quote(scriptID) + " of folder id " + NotesScript.quote(folder["scoped_alias"].string))
            }
            else {
                let content = args["mode"].string == "append" ? note["data"]["plain_text"].string + args["plain_text"].string : args["plain_text"].string
                _ = try NotesScript.run("set body of note id " + NotesScript.quote(scriptID) + " to " + NotesScript.quote(NotesScript.html(content)))
            }
        }
        for _ in 0..<100 {
            let snapshot = try store.read()
            let observed = snapshot.observations.first { $0["scoped_alias"].string == scriptID }
            if op == "notes.delete", observed == nil || observed?["data"]["recently_deleted"].bool == true {
                return .object(["native_key": action["native_key"], "observed": .bool(true), "absent": .bool(observed == nil)])
            }
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
