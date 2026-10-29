import Foundation
import Darwin
import SourceSupport

@MainActor
public final class VoiceMemosSource: Source {
    public nonisolated let name = "voice_memos"
    private let path: String
    private var generation = ""
    private var committed: [String: JSON] = [:]
    private var trackedAudio: Set<String> = []
    private var assetCache: [String: (String, JSON, [JSON])] = [:]
    private var folders: [String: JSON] = [:]
    private var assets: VoiceMemoAssets?
    public func readBlob(_ handle: String, offset: Int64, length: Int) throws -> JSON {
        guard let assets, offset >= 0, offset <= Int64(Int.max) else { throw SourceFailure.unavailable }
        return try assets.read(handle, offset: Int(offset), length: length)
    }
    public init(path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db").path) { self.path = path }
    private func reader() throws -> SQLiteSnapshot {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT Z_UUID FROM Z_METADATA")
        try reader.requireSchema(["ZCLOUDRECORDING": ["Z_PK","ZUNIQUEID","ZCUSTOMLABEL","ZDATE","ZDURATION","ZEVICTIONDATE","ZFOLDER"]])
        return reader
    }
    public func open() async -> SourceReadiness {
        guard FileManager.default.fileExists(atPath: path) else { return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unknown")]), reason: "store not present") }
        do { _ = try reader(); return SourceReadiness(state: "ready", coverage: .object(["fields": .string("metadata_and_local_assets"), "transcript": .string("embedded_if_present"), "audio": .string("local_if_present")])) }
        catch { return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unknown")]), reason: "store_access_or_schema_unavailable") }
    }
    public func scan(into sink: any ObservationSink, live: Bool, maxRows: Int = 100000) async throws {
        let started = timestamp()
        let reader = try reader()
        let columns = try reader.query("PRAGMA table_info(ZCLOUDRECORDING)").rows.compactMap { $0["name"]?.text }
        let assetColumn = columns.contains("ZPATH") ? ",ZPATH" : ""
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT COALESCE(NULLIF(ZUNIQUEID,''),CAST(Z_PK AS TEXT)) AS id,CAST(ZFOLDER AS TEXT) AS folder_id,ZUNIQUEID,ZCUSTOMLABEL,ZDATE,ZDURATION,ZEVICTIONDATE,ZFOLDER" + assetColumn + " FROM ZCLOUDRECORDING", keyColumn: "id", scope: "voice_memos.recording", previousScope: "voice_memos.recording", previousGeneration: generation, previousKeys: Set(committed.keys), maxRows: maxRows, timeout: 5)
        guard scan.complete else { throw SourceFailure.unavailable }
        var current: [String: JSON] = [:]
        var handles: [String: String] = [:]
        if assets == nil { assets = try VoiceMemoAssets(home: FileManager.default.temporaryDirectory.appendingPathComponent("apple-bridge-voice-memos-" + UUID().uuidString)) }
        for row in scan.rows {
            func number(_ key: String) -> Double {
                switch row[key] { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 }
            }
            guard let id = row["id"]?.text, number("ZDURATION").isFinite, number("ZDATE").isFinite, number("ZDURATION") >= 0 else { throw SourceFailure.corrupt }
            current[id] = .object(["title": .string(row["ZCUSTOMLABEL"]?.text ?? ""), "date": .string(timestamp(Date(timeIntervalSinceReferenceDate: number("ZDATE")))), "duration": .number(number("ZDURATION")), "folder": .string(row["folder_id"]?.text ?? "") , "recently_deleted": .bool(row["ZEVICTIONDATE"] != .null), "transcript_state": .string("unavailable"), "audio_state": .string("unavailable")])
            if let relative = row["ZPATH"]?.text, let file = localAudio(relative), let assets {
                trackedAudio.insert(file.path)
                let version = fileFingerprint(file.path)
                let cached = assetCache[file.path]
                let reference: JSON
                let segments: [JSON]
                if let cached, cached.0 == version {
                    reference = cached.1; segments = cached.2
                } else if let staged = try? await assets.stage(file), let copied = assets.file(staged["digest"].string) {
                    reference = staged
                    segments = (try? VoiceMemoTranscript.read(copied)) ?? []
                    assetCache[file.path] = (version, reference, segments)
                } else { continue }
                var data = current[id]!.object
                data["audio"] = reference
                data["audio_state"] = .string("present")
                if !segments.isEmpty { data["transcript"] = .array(segments); data["transcript_state"] = .string("present") }
                current[id] = .object(data)
                handles.merge(assets.handles([reference])) { _, new in new }

            }
        }
        var currentFolders: [String: JSON] = [:]
        let tables = try reader.query("SELECT name FROM sqlite_schema WHERE type='table' AND name='ZFOLDER'")
        if !tables.rows.isEmpty {
            let rows = try reader.query("SELECT Z_PK,ZENCRYPTEDNAME FROM ZFOLDER", maxRows: maxRows)
            guard rows.complete else { throw SourceFailure.unavailable }
            for row in rows.rows {
                guard case .integer(let key) = row["Z_PK"] else { throw SourceFailure.corrupt }
                let id = String(key)
                currentFolders[id] = .object(["name": .string(row["ZENCRYPTEDNAME"]?.text ?? ""), "count": .number(Double(current.values.filter { $0["folder"].string == id && !$0["recently_deleted"].bool }.count))])
            }
        }
        let same = generation == reader.generation
        var observations: [JSON] = []
        for id in current.keys.sorted() where !live || !same || current[id] != committed[id] {
            observations.append(.object(["kind": .string("upsert"), "collection": .string("voice_memos.recording"), "native_key": .string(id), "scoped_alias": .string("voice_memos/" + reader.generation + "/" + id), "data": current[id]!, "observed_at": .string(started)]))
        }
        for id in currentFolders.keys.sorted() where !live || !same || currentFolders[id] != folders[id] {
            observations.append(.object(["kind": .string("upsert"), "collection": .string("voice_memos.folder"), "native_key": .string(id), "scoped_alias": .string("voice_memos/folder/" + reader.generation + "/" + id), "data": currentFolders[id]!, "observed_at": .string(started)]))
        }
        for id in folders.keys.sorted() where same && currentFolders[id] == nil {
            observations.append(.object(["kind": .string("absent"), "collection": .string("voice_memos.folder"), "native_key": .string(id), "scoped_alias": .string("voice_memos/folder/" + reader.generation + "/" + id), "revalidated": .bool(true), "observed_at": .string(started)]))
        }
        for id in scan.absentKeys.sorted() {
            observations.append(.object(["kind": .string("absent"), "collection": .string("voice_memos.recording"), "native_key": .string(id), "scoped_alias": .string("voice_memos/" + reader.generation + "/" + id), "revalidated": .bool(true), "observed_at": .string(started)]))
        }
        try await sink.emitCheckpoint(source: name, generation: reader.generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["voice_memos.recording", "voice_memos.folder"], completionEvidence: "complete", blobHandles: handles, live: live && same)
        generation = reader.generation
        committed = current
        folders = currentFolders
    }
    private func localAudio(_ value: String) -> URL? {
        let root = URL(fileURLWithPath: path).deletingLastPathComponent().resolvingSymlinksInPath()
        let file = (value.hasPrefix("/") ? URL(fileURLWithPath: value) : root.appendingPathComponent(value)).resolvingSymlinksInPath()
        guard file.path.hasPrefix(root.path + "/"), file.pathExtension.lowercased() == "m4a" else { return nil }
        return file
    }
    private func fingerprint() -> String {
        let root = URL(fileURLWithPath: path).deletingLastPathComponent()
        let audio = ((try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []).filter { $0.pathExtension.lowercased() == "m4a" }.map(\.path).sorted()
        return Set([path, path + "-wal"] + audio + Array(trackedAudio)).sorted().map { fileFingerprint($0) }.joined(separator: "|")
    }
    private func fileFingerprint(_ file: String) -> String {
        var value = stat()
        guard stat(file, &value) == 0 else { return "missing" }
        return "\(value.st_dev):\(value.st_ino):\(value.st_size):\(value.st_mtimespec.tv_sec):\(value.st_mtimespec.tv_nsec):\(value.st_ctimespec.tv_sec):\(value.st_ctimespec.tv_nsec)"
    }

    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = DatabaseWatcher(path: path, pollInterval: .seconds(1))
        defer { watcher.close() }
        var observed = fingerprint()
        try await scan(into: sink, live: false)
        for await _ in watcher.changes {
            try Task.checkCancellation()
            let current = fingerprint()
            guard current != observed else { continue }
            try await scan(into: sink, live: true)
            observed = current
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
