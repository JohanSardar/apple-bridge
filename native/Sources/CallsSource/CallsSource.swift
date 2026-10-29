import AppKit
import Foundation
import Darwin
import SourceSupport

@MainActor
public final class CallsSource: Source {
    public nonisolated let name = "phone"
    private let path: String
 private let dialer: @MainActor @Sendable (URL) throws -> Bool
 private let observationWindow: Double
    private var generation = ""
    private var committed: [String: JSON] = [:]
    public init(path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/CallHistoryDB/CallHistory.storedata").path, observationWindow: Double = 10, dialer: @escaping @MainActor @Sendable (URL) throws -> Bool = { NSWorkspace.shared.open($0) }) { self.path = path; self.dialer = dialer; self.observationWindow = observationWindow }
    private func reader() throws -> SQLiteSnapshot {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT Z_UUID FROM Z_METADATA")
        try reader.requireSchema(["ZCALLRECORD": ["Z_PK", "ZADDRESS", "ZNAME", "ZDATE", "ZDURATION", "ZORIGINATED", "ZANSWERED", "ZSERVICE_PROVIDER", "ZCALLTYPE"]])
        return reader
    }
    public func open() async -> SourceReadiness {
        do { _ = try reader(); return SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete")]), capabilities: ["phone.dial"]) }
        catch { return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unknown")]), reason: "callhistory_access_or_schema_unavailable") }
    }
    public func scan(into sink: any ObservationSink, live: Bool, maxRows: Int = 100000) async throws {
        let started = timestamp()
        let reader = try reader()
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT CAST(Z_PK AS TEXT) AS id,ZADDRESS,ZNAME,ZDATE,ZDURATION,ZORIGINATED,ZANSWERED,ZSERVICE_PROVIDER,ZCALLTYPE FROM ZCALLRECORD", keyColumn: "id", scope: "phone.call", previousScope: "phone.call", previousGeneration: generation, previousKeys: Set(committed.keys), maxRows: maxRows, timeout: 5)
        guard scan.complete else { throw SourceFailure.unavailable }
        var current: [String: JSON] = [:]
        for row in scan.rows {
            func number(_ key: String) -> Double {
                switch row[key] { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 }
            }
            let originated = number("ZORIGINATED") != 0, answered = number("ZANSWERED") != 0
            let provider = row["ZSERVICE_PROVIDER"]?.text ?? ""
            let service: String
            if provider.lowercased().contains("facetime") { service = number("ZCALLTYPE") == 8 ? "FaceTime video" : "FaceTime audio" }
            else if provider.isEmpty || provider.lowercased() == "com.apple.telephony" { service = "cellular" }
            else { throw SourceFailure.unsupported }
            guard let id = row["id"]?.text, number("ZDURATION") >= 0 else { throw SourceFailure.corrupt }
            current[id] = .object(["direction": .string(originated ? "outgoing" : answered ? "incoming" : "missed"), "handle": .string(row["ZADDRESS"]?.text ?? ""), "name": .string(row["ZNAME"]?.text ?? ""), "date": .string(timestamp(Date(timeIntervalSinceReferenceDate: number("ZDATE")))), "duration": .number(number("ZDURATION")), "service": .string(service), "answered": .bool(answered)])
        }
        let same = generation == reader.generation
        var observations: [JSON] = []
        for id in current.keys.sorted() where !live || !same || current[id] != committed[id] {
            observations.append(.object(["kind": .string("upsert"), "collection": .string("phone.call"), "native_key": .string(id), "scoped_alias": .string("phone/" + reader.generation + "/" + id), "data": current[id]!, "observed_at": .string(started)]))
        }
        for id in scan.absentKeys.sorted() {
            observations.append(.object(["kind": .string("absent"), "collection": .string("phone.call"), "native_key": .string(id), "scoped_alias": .string("phone/" + reader.generation + "/" + id), "revalidated": .bool(true), "observed_at": .string(started)]))
        }
        try await sink.emitCheckpoint(source: name, generation: reader.generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["phone.call"], completionEvidence: "complete", blobHandles: [:], live: live && same)
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
    private func dialRows() throws -> [String: JSON] {
        let reader = try reader()
        let scan = try reader.query("SELECT CAST(Z_PK AS TEXT) AS id,ZADDRESS,ZDATE,ZORIGINATED FROM ZCALLRECORD")
        guard scan.complete else { throw SourceFailure.unavailable }
        let rows = scan.rows
        var result: [String: JSON] = [:]
        for row in rows {
            guard let id = row["id"]?.text else { throw SourceFailure.corrupt }
            func number(_ key: String) -> Double { switch row[key] { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 } }
            result[id] = .object(["number": .string(row["ZADDRESS"]?.text ?? ""), "date": .number(number("ZDATE")), "outgoing": .bool(number("ZORIGINATED") != 0)])
        }
        return result
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"], number = command["action"]["args"]["number"].string
        guard action["op"].string == "phone.dial", number.range(of: #"^\+?[0-9]{3,15}$"#, options: .regularExpression) != nil else { throw SourceFailure.invalid }
        _ = try dialRows()
        return Prepared(nativeState: .object(["route": .string("continuity_tel")]), fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let before = try dialRows(), started = Date()
        let number = prepared.action["args"]["number"].string
        guard let url = URL(string: "tel:" + number) else { throw SourceFailure.invalid }
        guard try dialer(url) else { return .object(["disposition": .string("indeterminate")]) }
        repeat {
            let rows = try dialRows()
            let candidates = rows.filter { id, row in
                before[id] == nil && row["outgoing"].bool && row["number"].string == number && row["date"].number >= started.timeIntervalSinceReferenceDate && row["date"].number <= started.timeIntervalSinceReferenceDate + observationWindow
            }
            if candidates.count == 1, let key = candidates.keys.first {
                return .object(["disposition": .string("locally_observed"), "observed": .bool(true), "native_key": .string(key)])
            }
            if candidates.count > 1 { break }
            try await Task.sleep(for: .milliseconds(100))
        } while Date().timeIntervalSince(started) < observationWindow
        return .object(["disposition": .string("indeterminate")])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
