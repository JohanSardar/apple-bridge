import Foundation
import Darwin
import SourceSupport

@MainActor
public final class VoiceMemosSource: Source {
    public nonisolated let name = "voice_memos"
    private let path: String
    private var generation = ""
    private var committed: [String: JSON] = [:]
    public init(path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db").path) { self.path = path }
    private func reader() throws -> SQLiteSnapshot {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT Z_UUID FROM Z_METADATA")
        try reader.requireSchema(["ZCLOUDRECORDING": ["Z_PK","ZUNIQUEID","ZCUSTOMLABEL","ZDATE","ZDURATION","ZEVICTIONDATE","ZFOLDER"]])
        return reader
    }
    public func open() async -> SourceReadiness {
        guard FileManager.default.fileExists(atPath: path) else { return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unknown")]), reason: "store not present") }
        do { _ = try reader(); return SourceReadiness(state: "ready", coverage: .object(["fields": .string("metadata_only"), "transcript": .string("unavailable"), "audio": .string("unavailable")])) }
        catch { return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unknown")]), reason: "store_access_or_schema_unavailable") }
    }
    public func scan(into sink: any ObservationSink, live: Bool, maxRows: Int = 100000) async throws {
        let started = timestamp()
        let reader = try reader()
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT COALESCE(NULLIF(ZUNIQUEID,''),CAST(Z_PK AS TEXT)) AS id,CAST(ZFOLDER AS TEXT) AS folder_id,* FROM ZCLOUDRECORDING", keyColumn: "id", scope: "voice_memos.recording", previousScope: "voice_memos.recording", previousGeneration: generation, previousKeys: Set(committed.keys), maxRows: maxRows, timeout: 5)
        guard scan.complete else { throw SourceFailure.unavailable }
        var current: [String: JSON] = [:]
        for row in scan.rows {
            func number(_ key: String) -> Double {
                switch row[key] { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 }
            }
            guard let id = row["id"]?.text, number("ZDURATION").isFinite, number("ZDATE").isFinite, number("ZDURATION") >= 0 else { throw SourceFailure.corrupt }
            current[id] = .object(["title": .string(row["ZCUSTOMLABEL"]?.text ?? ""), "date": .string(timestamp(Date(timeIntervalSinceReferenceDate: number("ZDATE")))), "duration": .number(number("ZDURATION")), "folder": .string(row["folder_id"]?.text ?? "") , "recently_deleted": .bool(row["ZEVICTIONDATE"] != .null), "transcript_state": .string("unavailable"), "audio_state": .string("unavailable")])
        }
        let same = generation == reader.generation
        var observations: [JSON] = []
        for id in current.keys.sorted() where !live || !same || current[id] != committed[id] {
            observations.append(.object(["kind": .string("upsert"), "collection": .string("voice_memos.recording"), "native_key": .string(id), "scoped_alias": .string("voice_memos/" + reader.generation + "/" + id), "data": current[id]!, "observed_at": .string(started)]))
        }
        for id in scan.absentKeys.sorted() {
            observations.append(.object(["kind": .string("absent"), "collection": .string("voice_memos.recording"), "native_key": .string(id), "scoped_alias": .string("voice_memos/" + reader.generation + "/" + id), "revalidated": .bool(true), "observed_at": .string(started)]))
        }
        try await sink.emitCheckpoint(source: name, generation: reader.generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["voice_memos.recording"], completionEvidence: "complete", blobHandles: [:], live: live && same)
        generation = reader.generation
        committed = current
    }
    private func fingerprint() -> String {
        [path, path + "-wal"].map { file in
            var value = stat()
            guard stat(file, &value) == 0 else { return "missing" }
            return "\(value.st_dev):\(value.st_ino):\(value.st_size):\(value.st_mtimespec.tv_sec):\(value.st_mtimespec.tv_nsec):\(value.st_ctimespec.tv_sec):\(value.st_ctimespec.tv_nsec)"
        }.joined(separator: "|")
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = DatabaseWatcher(path: path)
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
