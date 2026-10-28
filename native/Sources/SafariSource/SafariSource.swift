import Foundation
import SourceSupport
import CryptoKit

public actor SafariSource: Source {
    public nonisolated let name = "safari"
    private let directory: URL
    private let cacheURL: URL?
    private var generation = UUID().uuidString
    public init(directory: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Safari"), cacheURL: URL? = nil) { self.directory = directory; self.cacheURL = cacheURL }
    public func open() async -> SourceReadiness {
        guard FileManager.default.fileExists(atPath: directory.path) else {
            return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("partial")]), reason: "Safari data directory unavailable")
        }
        let readable = ["History.db", "Bookmarks.plist", "CloudTabs.db"].contains { FileManager.default.isReadableFile(atPath: directory.appendingPathComponent($0).path) }
        return SourceReadiness(state: readable ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("CloudTabs schema availability varies")])]), reason: readable ? "" : "Full Disk Access required")
    }
    private func observation(_ collection: String, _ key: String, _ data: [String: JSON]) -> JSON {
        .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(name + "/" + collection + "/" + key), "observed_at": .string(timestamp()), "data": .object(data)])
    }
    public func history(now: Date = Date(), days: Double = 90, maxRows: Int = 1_000_000) throws -> (String, [JSON]) {
        let reader = try SQLiteSnapshot(path: directory.appendingPathComponent("History.db").path, generationSQL: "SELECT 'safari-history-1'")
        try reader.requireSchema(["history_items": ["id", "url", "visit_count"], "history_visits": ["id", "history_item", "visit_time", "title"]])
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT CAST(i.id AS TEXT) AS key, i.url, COUNT(v.id) AS visit_count, MAX(v.visit_time) AS last_visit, (SELECT title FROM history_visits WHERE history_item=i.id ORDER BY visit_time DESC,id DESC LIMIT 1) AS title FROM history_items i JOIN history_visits v ON v.history_item=i.id WHERE v.visit_time >= ? GROUP BY i.id ORDER BY i.id", values: [String(now.timeIntervalSinceReferenceDate - days * 86400)], keyColumn: "key", scope: "safari.history", previousScope: "", previousGeneration: "", previousKeys: [], maxRows: maxRows, timeout: 15)
        guard scan.complete else { throw SourceFailure.unavailable }
        return (reader.generation, scan.rows.map { row in
            observation("safari.history", SHA256.hash(data: Data((row["url"]?.text ?? "").utf8)).map { String(format: "%02x", $0) }.joined(), ["url": .string(row["url"]?.text ?? ""), "title": .string(row["title"]?.text ?? ""), "last_visit": .string(timestamp(Date(timeIntervalSinceReferenceDate: number(row["last_visit"])))), "visit_count": .number(number(row["visit_count"]))])
        })
    }
    private func number(_ value: SQLiteValue?) -> Double {
        switch value { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 }
    }
    public func bookmarks() throws -> [JSON] {
        let path = directory.appendingPathComponent("Bookmarks.plist")
        let bytes = try Data(contentsOf: path)
        guard let root = try PropertyListSerialization.propertyList(from: bytes, format: nil) as? [String: Any], root["Children"] is [[String: Any]] else { throw SourceFailure.corrupt }
        var rows: [JSON] = []
        var keys = Set<String>()
        func walk(_ node: [String: Any], _ folders: [String], _ reading: Bool, _ depth: Int) throws {
            guard depth < 100 else { throw SourceFailure.corrupt }
            let type = node["WebBookmarkType"] as? String ?? ""
            let title = node["Title"] as? String ?? ""
            if type == "WebBookmarkTypeList" || node["Children"] != nil {
                let children: [[String: Any]]
                if let value = node["Children"] {
                    guard let parsed = value as? [[String: Any]] else { throw SourceFailure.corrupt }
                    children = parsed
                } else { children = [] }
                for child in children { try walk(child, title.isEmpty ? folders : folders + [title], reading || title == "com.apple.ReadingList", depth + 1) }
            } else if type == "WebBookmarkTypeLeaf" {
                guard let url = node["URLString"] as? String, let key = node["WebBookmarkUUID"] as? String, !key.isEmpty, keys.insert(key).inserted else { throw SourceFailure.corrupt }
                let name = (node["URIDictionary"] as? [String: Any])?["title"] as? String ?? title
                var data: [String: JSON] = ["url": .string(url), "title": .string(name)]
                if reading {
                    let metadata = node["ReadingList"] as? [String: Any] ?? [:]
                    data["added"] = (metadata["DateAdded"] as? Date).map { .string(timestamp($0)) } ?? .null
                    data["read"] = .bool(metadata["DateLastViewed"] != nil)
                } else { data["folder_path"] = .string(folders.joined(separator: "/")) }
                rows.append(observation(reading ? "safari.reading_list" : "safari.bookmark", key, data))
            } else if type != "WebBookmarkTypeProxy" { throw SourceFailure.corrupt }
        }
        try walk(root, [], false, 0)
        guard try Data(contentsOf: path) == bytes else { throw SourceFailure.conflict }
        return rows
    }
    public func cloudTabs() throws -> (String, [JSON]) {
        let reader = try SQLiteSnapshot(path: directory.appendingPathComponent("CloudTabs.db").path, generationSQL: "SELECT 'safari-cloud-1'")
        try reader.requireSchema(["cloud_tabs": ["tab_uuid", "device_uuid", "title", "url"], "cloud_tab_devices": ["device_uuid", "device_name"]])
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT t.tab_uuid AS key,t.title,t.url,d.device_name AS device FROM cloud_tabs t JOIN cloud_tab_devices d ON t.device_uuid=d.device_uuid ORDER BY t.tab_uuid", keyColumn: "key", scope: "safari.cloud_tab", previousScope: "", previousGeneration: "", previousKeys: [], maxRows: 100_000, timeout: 5)
        guard scan.complete else { throw SourceFailure.unavailable }
        return (reader.generation, scan.rows.map { row in observation("safari.cloud_tab", row["key"]!.text!, ["device": .string(row["device"]?.text ?? ""), "title": .string(row["title"]?.text ?? ""), "url": .string(row["url"]?.text ?? "")]) })
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let cached = try SafariCaptureCache.load(cacheURL)
        if !resume["generation"].string.isEmpty { generation = resume["generation"].string }
        else if let cached { generation = cached.generation }
        let watchers = ["History.db", "Bookmarks.plist", "CloudTabs.db"].map { DatabaseWatcher(path: directory.appendingPathComponent($0).path) }
        let events = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let tasks = watchers.map { watcher in Task { for await _ in watcher.changes { events.continuation.yield(()) } } }
        defer { watchers.forEach { $0.close() }; tasks.forEach { $0.cancel() }; events.continuation.finish() }
        var inputs: [String: String] = [:]
        var scannedAt: [String: Double] = [:]
        var previous: [String: String] = [:]
        var previousRows = cached?.rows ?? [:]
        var generations = cached?.scans.mapValues { $0.generation } ?? [:]
        events.continuation.yield(())
        for await _ in events.stream {
            try Task.checkCancellation()
            for kind in ["history", "bookmarks", "cloud"] {
                let input = safariInputs(directory, kind: kind)
                let uptime = ProcessInfo.processInfo.systemUptime
                if inputs[kind] == input, uptime - (scannedAt[kind] ?? 0) < 30 { continue }
                inputs[kind] = input
                scannedAt[kind] = uptime
                let started = timestamp()
                do {
                    let generation: String
                    let rows: [JSON]
                    let scope: [String]
                    switch kind {
                    case "history": (generation, rows) = try history(); scope = ["safari.history"]
                    case "cloud": (generation, rows) = try cloudTabs(); scope = ["safari.cloud_tab"]
                    default: rows = try bookmarks(); generation = "bookmarks-1"; scope = ["safari.bookmark", "safari.reading_list"]
                    }
                    let material = rows.map { value -> JSON in var object = value.object; object.removeValue(forKey: "observed_at"); return .object(object) }
                    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
                    let digest = SHA256.hash(data: try encoder.encode(material)).description
                    if previous[kind] != generation + digest {
                        let currentKeys = Set(rows.map { $0["collection"].string + "/" + $0["native_key"].string })
                        guard currentKeys.count == rows.count else { throw SourceFailure.corrupt }
                        let oldRows = previousRows[kind] ?? []
                        let oldData = oldRows.reduce(into: [String: JSON]()) { result, row in result[row["collection"].string + "/" + row["native_key"].string] = row["data"] }
                        let changes = rows.filter { oldData[$0["collection"].string + "/" + $0["native_key"].string] != $0["data"] }
                        var pendingRows = previousRows
                        pendingRows[kind] = rows + oldRows.filter { !currentKeys.contains($0["collection"].string + "/" + $0["native_key"].string) }
                        var pendingGenerations = generations
                        pendingGenerations[kind] = generation
                        try SafariCaptureCache.save(generation: self.generation, databaseGenerations: pendingGenerations, rows: pendingRows, to: cacheURL)
                        let absent = generations[kind] == generation ? (previousRows[kind] ?? []).filter { !currentKeys.contains($0["collection"].string + "/" + $0["native_key"].string) }.map { row in JSON.object(["kind": .string("absent"), "collection": row["collection"], "native_key": row["native_key"], "observed_at": .string(timestamp()), "revalidated": .bool(true)]) } : []
                        try await sink.emitCheckpoint(source: name, generation: self.generation, checkpoint: UUID().uuidString, observations: changes + absent, scanID: UUID().uuidString, scanStartedAt: started, scope: scope, completionEvidence: "complete", blobHandles: [:], live: previous[kind] != nil)
                        let replaced = generations[kind] != nil && generations[kind] != generation
                        let carried = replaced ? (previousRows[kind] ?? []).filter { !currentKeys.contains($0["collection"].string + "/" + $0["native_key"].string) } : []
                        previous[kind] = carried.isEmpty ? generation + digest : ""
                        previousRows[kind] = rows + carried
                        generations[kind] = generation
                        try SafariCaptureCache.save(generation: self.generation, databaseGenerations: generations, rows: previousRows, to: cacheURL)
                        if !carried.isEmpty { inputs.removeValue(forKey: kind); events.continuation.yield(()) }
                    }
                } catch is CancellationError { throw CancellationError() }
                catch { try await sink.sourceState(source: name, state: "degraded", coverage: .object(["fields": .string("partial")]), capabilities: [], reason: "Safari capture unavailable: " + kind) }
            }
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
