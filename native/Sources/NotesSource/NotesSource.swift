import Foundation
import CryptoKit
import SourceSupport
import MediaDerivatives

@MainActor
public final class NotesSource: Source {
    public nonisolated let name = "notes"
    public static let operations = ["notes.create", "notes.update", "notes.delete", "notes.create_folder", "notes.move", "notes.rename_folder"]
    let store: NotesStore
    let deletionProofURL: URL
    private let baselineURL: URL
    private let assets: NotesAssets
    private let trace: CaptureTrace
    private let changes: NotesChangeStore
    private struct Hydration {
        let key: String
        let fingerprint: JSON
        let scanID: String
    }
    private var pendingHydration: [String: Hydration] = [:]
    private var hydrationOrder: [String] = []
    private var hydrationTask: Task<Void, Never>?
    private var hydrationID = ""
    private var activeHydration: Hydration?
    private var watcher: NotesWatcher?
    private var notePaths: [String: Set<String>] = [:]
    #if LAB
    public private(set) var lastChangedNotes = 0
    public private(set) var bootstrapScans = 0
    public func waitForHydration() async throws {
        while hydrationTask != nil { try await Task.sleep(for: .milliseconds(5)) }
    }
    #endif
    private var previous: [String: JSON] = [:]
    private var generation = ""
    private var scanning = false
    public convenience init(home: URL, trace: CaptureTrace = CaptureTrace()) throws {
        try self.init(home: home, database: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite"), trace: trace)
    }
    #if LAB
    public convenience init(home: URL, fixtureDatabase: URL, trace: CaptureTrace = CaptureTrace()) throws {
        try self.init(home: home, database: fixtureDatabase, trace: trace)
    }
    #endif
    private init(home: URL, database: URL, trace: CaptureTrace) throws {
        self.trace = trace
        assets = try NotesAssets(home: home)
        store = NotesStore(path: database.path)
        deletionProofURL = home.appendingPathComponent("notes-deletion-proofs.json")
        changes = NotesChangeStore(path: database.path)
        baselineURL = home.appendingPathComponent("notes-identities.json")
        if let bytes = try? Data(contentsOf: baselineURL), let saved = try? JSONDecoder().decode(JSON.self, from: bytes) {
            generation = saved["generation"].string
            for item in saved["identities"].array { previous[item["collection"].string + "/" + item["native_key"].string] = item }
        }
    }
    public func open() async -> SourceReadiness {
        let readable = (try? NotesDatabase(path: store.path).query("SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME=?", ["ICNote"])) != nil
        let proven = deletionCapabilities(), extras = extrasCapabilities()
        let operationCoverage = (Self.deletionOperations + Self.extrasOperations).map { operation in
            JSON.object(["operation": .string(operation), "proven": .bool(proven.contains(operation) || extras.contains(operation)), "reason": .string(operation == "notes.delete" ? "public_script_notestore_verified" : proven.contains(operation) ? "os_build_notestore_deletion_verified" : extras.contains(operation) ? "public_script_move_rename" : "capability_unproven")])
        }
        return SourceReadiness(state: readable ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "operations": .array(operationCoverage), "missing": .array([.string("unknown protobuf fields and unavailable attachment bytes are explicit; rich edits remain unsupported; folder deletion requires per-build proof")])]), capabilities: readable ? Self.operations.filter { (!Self.deletionOperations.contains($0) || proven.contains($0)) && (!Self.extrasOperations.contains($0) || extras.contains($0)) } : [], reason: readable ? "" : "notes_database_access_required")
    }
    public func reconcile(into sink: any ObservationSink, assetPaths: Set<String> = []) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        scanning = true
        defer { scanning = false }
        let scanID = UUID().uuidString, started = timestamp()
        let metadata: [String: JSON] = ["source": .string(name), "scan_id": .string(scanID)]
        trace.record("notes_scan_requested", fields: metadata)
        trace.record("notes_scan_started", fields: metadata)
        let scan = try changes.scan()
        trace.record("notes_store_read", fields: metadata.merging(["changed_notes": .number(Double(scan.changedNotes)), "bootstrap": .bool(scan.bootstrap)]) { _, new in new })
        #if LAB
        lastChangedNotes = scan.changedNotes
        if scan.bootstrap { bootstrapScans += 1 }
        #endif
        if generation != scan.snapshot.generation { previous = [:]; generation = scan.snapshot.generation }
        var records = previous, jobs: [Hydration] = []
        for observation in scan.snapshot.observations {
            let key = observation["collection"].string + "/" + observation["native_key"].string
            var value = observation.object
            if observation["collection"].string == "notes.note" {
                if let old = previous[key], old["data"]["native_fingerprint"] == observation["data"]["native_fingerprint"] {
                    value["data"] = old["data"]
                } else {
                    value["data"] = .object(NotesBodyProjection.pending(observation["data"].object))
                }
                if value["data"]?["locked"].bool != true, value["data"]?["body_status"].string == "pending" {
                    jobs.append(Hydration(key: observation["native_key"].string, fingerprint: observation["data"]["native_fingerprint"], scanID: scanID))
                }
            }
            records[key] = .object(value)
        }
        var removed = scan.removed
        if scan.bootstrap { removed.formUnion(Set(previous.keys).subtracting(Set(scan.snapshot.observations.map { $0["collection"].string + "/" + $0["native_key"].string }))) }
        for key in removed { records.removeValue(forKey: key); notePaths.removeValue(forKey: String(key.dropFirst("notes.note/".count))); pendingHydration.removeValue(forKey: String(key.dropFirst("notes.note/".count))) }
        var observations = scan.snapshot.observations.compactMap { item -> JSON? in
            let key = item["collection"].string + "/" + item["native_key"].string
            guard let value = records[key], previous[key]?["data"] != value["data"] else { return nil }
            return value
        }
        for key in removed {
            if let item = previous[key] { observations.append(.object(["kind": .string("absent"), "collection": item["collection"], "native_key": item["native_key"], "observed_at": .string(timestamp()), "revalidated": .bool(true)])) }
        }
        trace.record("notes_metadata_projected", fields: metadata)
        trace.record("notes_capture_ready", keys: observations.map { $0["native_key"].string }, fields: metadata)
        try await sink.emit(source: name, generation: generation, observations: Array(observations), scanID: scanID, scanStartedAt: started, scope: ["notes.folder", "notes.note"], completionEvidence: "complete", blobHandles: [:], live: true)
        trace.record("notes_capture_committed", fields: metadata)
        let identitiesChanged = scan.bootstrap || !removed.isEmpty || records.keys.contains { previous[$0] == nil }
        previous = records
        changes.accept(scan)
        for job in jobs { enqueue(job, priority: !scan.bootstrap, into: sink) }
        for record in records.values where record["collection"].string == "notes.note" && !record["data"]["locked"].bool {
            let data = record["data"]
            if data["body_status"].string == "pending" || data["body_status"].string == "unavailable" || data["attachments"].array.contains(where: { $0["state"].string == "unavailable" && $0["reason"].string != "attachment_metadata_unavailable" }) {
                enqueue(Hydration(key: record["native_key"].string, fingerprint: data["native_fingerprint"], scanID: scanID), priority: false, into: sink)
            }
        }
        for (key, paths) in notePaths where paths.contains(where: { path in assetPaths.contains(where: { path == $0 || path.hasPrefix($0 + "/") }) }) {
            if let record = records["notes.note/" + key], !record["data"]["locked"].bool {
                enqueue(Hydration(key: key, fingerprint: record["data"]["native_fingerprint"], scanID: scanID), priority: false, force: true, into: sink)
            }
        }
        if identitiesChanged {
            let identities = records.values.map { JSON.object(["collection": $0["collection"], "native_key": $0["native_key"]]) }
            try JSONEncoder().encode(JSON.object(["generation": .string(generation), "identities": .array(identities)])).write(to: baselineURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
        }
    }
    private func enqueue(_ job: Hydration, priority: Bool, force: Bool = false, into sink: any ObservationSink) {
        if !force, activeHydration?.key == job.key, activeHydration?.fingerprint == job.fingerprint { return }
        if !force, pendingHydration[job.key]?.fingerprint == job.fingerprint { return }
        pendingHydration[job.key] = job
        if priority { hydrationOrder.removeAll { $0 == job.key }; hydrationOrder.insert(job.key, at: 0) }
        else if !hydrationOrder.contains(job.key) { hydrationOrder.append(job.key) }
        guard hydrationTask == nil else { return }
        let workerID = UUID().uuidString
        hydrationID = workerID
        hydrationTask = Task { [weak self] in
            guard let self else { return }
            defer { if self.hydrationID == workerID { self.hydrationTask = nil } }
            while !Task.isCancelled, self.hydrationID == workerID, !self.hydrationOrder.isEmpty {
                let key = self.hydrationOrder.removeFirst()
                guard let job = self.pendingHydration.removeValue(forKey: key) else { continue }
                #if LAB
                while FileManager.default.fileExists(atPath: self.baselineURL.deletingLastPathComponent().appendingPathComponent("lab-notes-hold-hydration").path), !Task.isCancelled { try? await Task.sleep(for: .milliseconds(5)) }
                #endif
                self.activeHydration = job
                do { try await self.hydrate(job, into: sink) }
                catch {
                    self.trace.record("notes_hydration_deferred", fields: ["source": .string(self.name), "scan_id": .string(job.scanID)])
                    if !Task.isCancelled { await self.hydrationFailed(job, into: sink) }
                }
                if self.hydrationID == workerID { self.activeHydration = nil }
            }
            try? self.assets.prune(self.previous.values.flatMap { NotesBodyProjection.references($0["data"].object) })
        }
    }
    private func hydrate(_ job: Hydration, into sink: any ObservationSink) async throws {
        let metadata: [String: JSON] = ["source": .string(name), "scan_id": .string(job.scanID)]
        trace.record("notes_hydration_started", fields: metadata)
        let path = store.path, key = job.key
        let read = Task.detached(priority: .utility) { try NotesStore(path: path).read(keys: [key]) }
        let snapshot = try await withTaskCancellationHandler(operation: { try await read.value }, onCancel: { read.cancel() })
        guard snapshot.generation == generation, let observation = snapshot.observations.first(where: { $0["collection"].string == "notes.note" && $0["native_key"].string == key }), observation["data"]["native_fingerprint"] == job.fingerprint else { watcher?.request(); return }
        var data = observation["data"].object
        guard data["locked"]?.bool != true else { watcher?.request(); return }
        var attachments: [JSON] = []
        for metadata in data["attachments", default: .array([])].array {
            var item = metadata.object
            if let attachment = snapshot.attachments[observation["native_key"].string + ":" + metadata["id"].string], let file = attachment.file {
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
            } else if let payload = snapshot.attachments[observation["native_key"].string + ":" + metadata["id"].string]?.payload {
                do { item["blob"] = try await assets.stage(payload); item["state"] = .string("present") }
                catch { item["state"] = .string("unavailable"); item["reason"] = .string("attachment_staging_unavailable") }
            }
            attachments.append(.object(item))
        }
        data["attachments"] = .array(attachments)
        if let bytes = snapshot.rawBodies[observation["native_key"].string] {
            do { data["raw_body"] = try await assets.stage(bytes) }
            catch {
                data["raw_body"] = .object(["digest": data["raw_body_digest"] ?? .null, "size": .number(Double(bytes.count)), "state": .string("missing"), "reason": .string("native_staging_unavailable")])
            }
        }
        data = try await NotesBodyProjection.fit(data, assets: assets)
        try Task.checkCancellation()
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        let recordKey = "notes.note/" + key
        guard previous[recordKey]?["data"]["native_fingerprint"] == job.fingerprint else { return }
        notePaths[key] = Set(snapshot.attachments.filter { $0.key.hasPrefix(key + ":") }.values.compactMap { $0.file?.standardizedFileURL.path })
        var value = observation.object; value["data"] = .object(data)
        trace.record("notes_hydration_finished", fields: metadata)
        if previous[recordKey]?["data"] != value["data"] {
            try await sink.emit(source: name, generation: generation, observations: [.object(value)], scanID: job.scanID + "-hydrate", scanStartedAt: timestamp(), scope: [], completionEvidence: "", blobHandles: assets.handles(NotesBodyProjection.references(data)), live: false)
            if previous[recordKey]?["data"]["native_fingerprint"] == job.fingerprint { previous[recordKey] = .object(value) }
        }
    }
    private func hydrationFailed(_ job: Hydration, into sink: any ObservationSink) async {
        while scanning && !Task.isCancelled { try? await Task.sleep(for: .milliseconds(5)) }
        let key = "notes.note/" + job.key
        guard !Task.isCancelled, let record = previous[key], record["data"]["native_fingerprint"] == job.fingerprint else { return }
        var data = record["data"].object, value = record.object
        data["body_status"] = .string("unavailable"); data["body_reason"] = .string("native_hydration_unavailable")
        value["data"] = .object(data)
        do {
            try await sink.emit(source: name, generation: generation, observations: [.object(value)], scanID: job.scanID + "-unavailable", scanStartedAt: timestamp(), scope: [], completionEvidence: "", blobHandles: [:], live: false)
            if previous[key]?["data"]["native_fingerprint"] == job.fingerprint { previous[key] = .object(value) }
        } catch { }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = NotesWatcher(path: URL(fileURLWithPath: store.path).deletingLastPathComponent().path, wal: store.path + "-wal", trace: trace)
        self.watcher = watcher
        defer { watcher.close(); self.watcher = nil; hydrationTask?.cancel(); hydrationTask = nil; hydrationID = ""; activeHydration = nil; pendingHydration = [:]; hydrationOrder = [] }
        for await _ in watcher.changes {
            try Task.checkCancellation()
            try await reconcile(into: sink, assetPaths: watcher.takePaths())
        }
    }
    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard let access = try? FileHandle(forReadingFrom: URL(fileURLWithPath: store.path)) else { throw SourceFailure.permissionMissing }
        try access.close()
        return try assets.read(handle, offset: offset, length: length)
    }
    private func find(_ key: String, collection: String = "notes.note") throws -> JSON {
        guard let note = try store.read(keys: [key]).observations.first(where: { $0["native_key"].string == key && $0["collection"].string == collection }) else { throw SourceFailure.conflict }
        return note
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        try requireDeletionProof(action["op"].string)
        guard Self.operations.contains(action["op"].string) else { throw SourceFailure.unsupported }
        var before = JSON.null
        if !action["native_key"].string.isEmpty {
            let note = try find(action["native_key"].string, collection: action["op"].string == "notes.rename_folder" ? "notes.folder" : "notes.note")
            before = note["data"]
            if action["op"].string == "notes.update" {
                if action["note_args"]["html"] != .null {
                    try NotesRichEdit.validate(before, html: action["note_args"]["html"].string)
                } else if before["editable"].string != "plain" { throw SourceFailure.unsupported }
            }
            guard !before["locked"].bool, !before["read_only"].bool, !before["recently_deleted"].bool else { throw SourceFailure.unsupported }
            if action["op"].string == "notes.rename_folder" {
                guard before["title"].string.hasPrefix("Bridge Test [ab-test"), argsTitle(action).hasPrefix("Bridge Test [ab-test"), command["expected_state"]["title"] == before["title"] else { throw SourceFailure.conflict }
                let bytes = try JSONEncoder().encode(action)
                return Prepared(nativeState: .object(["native_fingerprint": .string(try folderFingerprint(before))]), fingerprint: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(), action: action)
            }
            guard command["expected_state"]["native_fingerprint"] == before["native_fingerprint"] else { throw SourceFailure.conflict }
            let fingerprint = before["native_fingerprint"], text = before["plain_text"]
            before = .object(["native_fingerprint": fingerprint])
            if action["op"].string == "notes.update" { before = .object(["native_fingerprint": fingerprint, "plain_text": text]) }
        }
        if ["notes.create", "notes.move"].contains(action["op"].string) {
            let folder = try find(action["folder_key"].string, collection: "notes.folder")
            guard !folder["data"]["read_only"].bool else { throw SourceFailure.unsupported }
        }
        let bytes = try JSONEncoder().encode(action)
        return Prepared(nativeState: before, fingerprint: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(), action: action)
    }
    private func argsTitle(_ action: JSON) -> String { action["note_args"]["title"].string }
    private func folderFingerprint(_ data: JSON) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return SHA256.hash(data: try encoder.encode(data)).map { String(format: "%02x", $0) }.joined()
    }
    public func validate(_ prepared: Prepared) throws {
        if prepared.action["op"].string == "notes.rename_folder" {
            let folder = try find(prepared.action["native_key"].string, collection: "notes.folder")
            guard try folderFingerprint(folder["data"]) == prepared.nativeState["native_fingerprint"].string else { throw SourceFailure.conflict }; return
        }
        try requireDeletionProof(prepared.action["op"].string)
        if !prepared.action["native_key"].string.isEmpty {
            guard try find(prepared.action["native_key"].string)["data"]["native_fingerprint"] == prepared.nativeState["native_fingerprint"] else { throw SourceFailure.conflict }
        }
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        try validate(prepared)
        let action = prepared.action, args = action["note_args"]
        let op = action["op"].string
        var scriptID = ""
        if op == "notes.rename_folder" {
            let folder = try find(action["native_key"].string, collection: "notes.folder")
            scriptID = folder["scoped_alias"].string
            _ = try NotesScript.run("set name of folder id " + NotesScript.quote(scriptID) + " to " + NotesScript.quote(args["title"].string))
        } else if op == "notes.create_folder" {
            scriptID = try NotesScript.run("return id of (make new folder with properties {name:" + NotesScript.quote(args["title"].string) + "})")
        } else if op == "notes.create" {
            let folder = try find(action["folder_key"].string, collection: "notes.folder")
            let content = args["title"].string + "\n" + args["plain_text"].string
            scriptID = try NotesScript.run("return id of (make new note at folder id " + NotesScript.quote(folder["scoped_alias"].string) + " with properties {body:" + NotesScript.quote(NotesScript.html(content)) + "})")
        } else {
            let note = try find(action["native_key"].string)
            scriptID = note["scoped_alias"].string
            if op == "notes.move" {
                let folder = try find(action["folder_key"].string, collection: "notes.folder")
                _ = try NotesScript.run("move note id " + NotesScript.quote(scriptID) + " to folder id " + NotesScript.quote(folder["scoped_alias"].string))
            } else if op == "notes.delete" {
                let folder = try find(note["data"]["folder_id"]["$ref"]["native_key"].string, collection: "notes.folder")
                trace.record("notes_delete_started", keys: [action["native_key"].string], fields: ["source": .string(name)])
                let trash = try store.trashFolder(forFolder: folder["native_key"].string)
                _ = try NotesScript.run(NotesScript.deleteNote(scriptID, folderID: folder["scoped_alias"].string, trashFolderID: trash?["scoped_alias"].string), trace: trace)
            }
            else {
                let content = args["mode"].string == "append" ? note["data"]["plain_text"].string + args["plain_text"].string : args["plain_text"].string
                let html = args["html"] == .null ? NotesScript.html(content) : args["html"].string
                _ = try NotesScript.run("set body of note id " + NotesScript.quote(scriptID) + " to " + NotesScript.quote(html))
            }
        }
        for _ in 0..<100 {
            let snapshot = try store.read(nativeID: scriptID)
            let observed = snapshot.observations.first { $0["scoped_alias"].string == scriptID }
            if op == "notes.delete", try ["gone", "marked", "trashed"].contains(store.deletionState(action["native_key"].string, collection: "notes.note")) {
                trace.record("notes_delete_verified", keys: [action["native_key"].string], fields: ["source": .string(name)])
                return .object(["native_key": action["native_key"], "observed": .bool(true), "absent": .bool(observed == nil)])
            }
            if let observed, op != "notes.delete" {
                if op == "notes.rename_folder", observed["data"]["title"] != args["title"] {
                    try await Task.sleep(for: .milliseconds(50)); continue
                }
                if op == "notes.move", observed["data"]["folder_id"]["$ref"]["native_key"] != action["folder_key"] {
                    try await Task.sleep(for: .milliseconds(50)); continue
                }
                if op == "notes.update", args["html"] != .null {
                    if observed["data"]["html"] == args["html"] {
                        return .object(["native_key": observed["native_key"], "observed": .bool(true)])
                    }
                } else if op != "notes.update" || observed["data"]["plain_text"] != prepared.nativeState["plain_text"] {
                    return .object(["native_key": observed["native_key"], "observed": .bool(true)])
                }
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        if op == "notes.delete" { trace.record("notes_delete_unverified", keys: [action["native_key"].string], fields: ["source": .string(name)]) }
        throw SourceFailure.unavailable
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
