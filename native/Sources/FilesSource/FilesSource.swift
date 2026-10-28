import Foundation
import SourceSupport

public actor FilesSource: Source {
    public nonisolated let name = "files"
    let roots: [URL]
    private let writer: BlobWriter
    private var blobs: [String: URL] = [:]
    private var scanGuard = FileScanGuard()
    private var reconciling = false
    private let generation = UUID().uuidString
    public init(home: URL = FileManager.default.homeDirectoryForCurrentUser, roots: [URL] = []) throws {
        let defaultRoot = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
        try FileManager.default.createDirectory(at: defaultRoot, withIntermediateDirectories: true)
        let config = home.appendingPathComponent("Library/Application Support/AppleBridge/files-roots.json")
        let configured = FileManager.default.fileExists(atPath: config.path) ? try JSONDecoder().decode([String].self, from: Data(contentsOf: config)) : []
        guard configured.count + roots.count <= 16, configured.allSatisfy({ $0.hasPrefix("/") }) else { throw SourceFailure.invalid }
        self.roots = ([defaultRoot] + roots + configured.map { URL(fileURLWithPath: $0) }).map { $0.standardizedFileURL.resolvingSymlinksInPath() }
        writer = try BlobWriter(directory: home.appendingPathComponent("Library/Caches/AppleBridge/files-blobs"))
    }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["roots": .number(Double(roots.count)), "max_items": .number(10000), "max_depth": .number(16)]), capabilities: ["files.search", "files.read"] + Self.commands)
    }
    public func snapshot(_ root: URL) throws -> ([JSON], Bool) {
        guard roots.contains(root.standardizedFileURL.resolvingSymlinksInPath()) else { throw SourceFailure.invalid }
        var complete = true
        var failed = false
        let keys: Set<URLResourceKey> = [.isSymbolicLinkKey, .isDirectoryKey, .fileSizeKey, .creationDateKey, .contentModificationDateKey, .tagNamesKey, .ubiquitousItemDownloadingStatusKey, .isUbiquitousItemKey]
        guard let tree = FileManager.default.enumerator(at: root, includingPropertiesForKeys: Array(keys), options: [], errorHandler: { _, _ in failed = true; return false }) else { throw SourceFailure.unavailable }
        var rows: [JSON] = []
        var visited = 0
        while let url = tree.nextObject() as? URL {
            try Task.checkCancellation()
            visited += 1
            if visited > 10000 { complete = false; break }
            let path = String(url.path.dropFirst(root.path.count + 1))
            if path.split(separator: "/").count > 16 { tree.skipDescendants(); complete = false; continue }
            let v = try url.resourceValues(forKeys: keys)
            if v.isSymbolicLink == true { tree.skipDescendants(); continue }
            let data: JSON = .object(["root": .string(root.path), "path": .string(path), "name": .string(url.lastPathComponent), "kind": .string(v.isDirectory == true ? "directory" : url.pathExtension), "size": .number(Double(v.fileSize ?? 0)), "created": v.creationDate.map { .string(timestamp($0)) } ?? .null, "modified": v.contentModificationDate.map { .string(timestamp($0)) } ?? .null, "tags": .array((v.tagNames ?? []).map(JSON.string)), "downloaded": .bool(v.ubiquitousItemDownloadingStatus != .notDownloaded), "icloud_status": .string(v.ubiquitousItemDownloadingStatus?.rawValue ?? "local")])
            rows.append(.object(["kind": .string("upsert"), "collection": .string("files.item"), "native_key": .string(url.path), "observed_at": .string(timestamp()), "data": data]))
        }
        if failed { complete = false }
        return (rows, complete)
    }
    public func reconcile(into sink: any ObservationSink) async throws {
        while reconciling { try await Task.sleep(for: .milliseconds(25)) }
        reconciling = true
        defer { reconciling = false }
        for root in roots {
            let started = timestamp()
            let (scanned, complete) = try snapshot(root)
            var rows = scanned
            var nextGuard = scanGuard
            let absent = nextGuard.finish(root: root.path, keys: Set(rows.map { $0["native_key"].string }), complete: complete)
            rows += absent.map { .object(["kind": .string("absent"), "collection": .string("files.item"), "native_key": .string($0), "observed_at": .string(timestamp()), "revalidated": .bool(true)]) }
            try await sink.emit(source: name, generation: generation, observations: rows, scanID: UUID().uuidString, scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: [:], live: false)
            scanGuard = nextGuard
        }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = DatabaseWatcher(path: roots[0].path, coalescing: .milliseconds(250))
        defer { watcher.close() }
        repeat {
            try await reconcile(into: sink)
            var iterator = watcher.changes.makeAsyncIterator()
            guard await iterator.next() != nil else { return }
        } while !Task.isCancelled
    }
    public func read(_ args: JSON) async throws -> JSON {
        let root = URL(fileURLWithPath: args["root"].string).standardizedFileURL.resolvingSymlinksInPath()
        guard roots.contains(root), !args["path"].string.isEmpty else { throw SourceFailure.invalid }
        let url = root.appendingPathComponent(args["path"].string).standardizedFileURL.resolvingSymlinksInPath()
        guard url.path.hasPrefix(root.path + "/") else { throw SourceFailure.invalid }
        let values = try url.resourceValues(forKeys: [.isUbiquitousItemKey])
        if values.isUbiquitousItem == true {
            try FileManager.default.startDownloadingUbiquitousItem(at: url)
            let deadline = Date().addingTimeInterval(50)
            while try url.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]).ubiquitousItemDownloadingStatus == .notDownloaded {
                guard Date() < deadline else { throw SourceFailure.unavailable }
                try await Task.sleep(for: .milliseconds(100))
            }
        }
        let writer = self.writer
        var staged: StagedBlob?
        var failure: Error?
        var coordinationError: NSError?
        NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordinationError) { coordinated in
            do { staged = try writer.stage(coordinated) } catch { failure = error }
        }
        if let failure { throw failure }
        if let coordinationError { throw coordinationError }
        guard let blob = staged else { throw SourceFailure.unavailable }
        blobs[blob.digest] = blob.file
        return .object(["digest": .string(blob.digest), "size": .number(Double(blob.size))])
    }
    public func search(_ args: JSON) async throws -> JSON {
        guard !args["query"].string.isEmpty, args["query"].string.utf8.count <= 4096 else { throw SourceFailure.invalid }
        let field = args["field"].string.isEmpty ? "name" : args["field"].string
        let attributes = ["name": "kMDItemFSName", "content": "kMDItemTextContent", "kind": "kMDItemContentType", "tag": "kMDItemUserTags", "date": "kMDItemFSContentChangeDate"]
        guard let attribute = attributes[field] else { throw SourceFailure.invalid }
        let value = args["query"].string.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let predicate = attribute + " == \"" + value + "\"cd"
        let output = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        _ = FileManager.default.createFile(atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
        defer { try? FileManager.default.removeItem(at: output) }
        let handle = try FileHandle(forWritingTo: output)
        defer { try? handle.close() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/mdfind")
        process.arguments = ["-0"] + roots.flatMap { ["-onlyin", $0.path] } + [predicate]
        process.standardOutput = handle
        process.standardError = FileHandle.nullDevice
        try process.run()
        defer { if process.isRunning { process.terminate() } }
        let deadline = Date().addingTimeInterval(20)
        while process.isRunning {
            try Task.checkCancellation()
            guard Date() < deadline, (try output.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) < 1 << 20 else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard process.terminationStatus == 0 else { throw SourceFailure.unavailable }
        let paths = try Data(contentsOf: output).split(separator: 0).compactMap { String(data: $0, encoding: .utf8) }
        let items = paths.prefix(100).filter { path in roots.contains { path.hasPrefix($0.path + "/") } }.map { JSON.object(["path": .string($0)]) }
        return .object(["items": .array(items)])
    }
    public func readBlob(_ handle: String, offset: Int64, length: Int) throws -> JSON {
        let digest = String(handle.dropFirst("files-blob:".count))
        guard handle.hasPrefix("files-blob:"), let url = blobs[digest], offset >= 0, length > 0, length <= 131072 else { throw SourceFailure.invalid }
        let input = try FileHandle(forReadingFrom: url)
        defer { try? input.close() }
        try input.seek(toOffset: UInt64(offset))
        let bytes = try input.read(upToCount: length) ?? Data()
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(digest), "eof": .bool(offset + Int64(bytes.count) == Int64(size))])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
