import Foundation
import Decoders
import SourceSupport

public actor Messages: Source {
    public nonisolated let name = "messages"
    private let path: String
    private let diagnostics: URL?
    private var reader: MessagesReader?
    private var baseline: MessagesBaseline?
    private var candidates: Set<String> = []
    private var generation = ""
    private var hashes: [String: String] = [:]
    private var nativeHashes: [String: String] = [:]
    private var hashTimes: [String: String] = [:]
    private var nativeHashTimes: [String: String] = [:]
    private var pendingHashes: [String: [UUID: String]] = [:]
    private var pendingNativeHashes: [String: [UUID: String]] = [:]
    private var smallNativeHashes: [String: (String, String)] = [:]
    private var firstSmallSnapshot: ([(String, String, [String: Any])], String)?
    private var observationMillisecond: Int64 = 0
    private var captureActive = false
    private var reconciliationTask: Task<Void, Never>?
    private var reconciliationID: UUID?
    private var reconcileAgain = false
    private var lastRowID: Int64 = 0
    private var lastLiveVersion: Int64?
    private var lastComplete = Date.distantPast
    private var blobDigests: [String: (Int64, Date, String)] = [:]
    private var pendingBlobs: [String: URL] = [:]
    private var failedBlobs: Set<String> = []
    public init(path: String? = nil, diagnostics: URL? = nil) {
        self.diagnostics = diagnostics
        self.path = path ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
    }
    public func open() async -> SourceReadiness {
        do {
            guard FileManager.default.isReadableFile(atPath: path) else {
                return SourceReadiness(state: "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("chat_db_unavailable")])]), reason: "chat_db_unavailable")
            }
            if captureActive {
                guard let reader, try reader.isCurrent() else {
                    return SourceReadiness(state: "degraded", coverage: coverage(), reason: "reconciliation_pending")
                }
            } else { _ = try reopen() }
            return SourceReadiness(state: "ready", coverage: coverage())
        } catch { return SourceReadiness(state: FileManager.default.isReadableFile(atPath: path) ? "degraded" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("chat_db_unavailable")])]), reason: "chat_db_unavailable") }
    }
    private func coverage() -> JSON {
        let expected = [
            "message": ["guid", "date", "text", "attributedBody", "handle_id", "is_from_me", "date_delivered", "date_read", "is_delivered", "is_read", "error", "is_finished", "item_type", "date_edited", "date_retracted", "message_summary_info", "associated_message_guid", "associated_message_type", "associated_message_emoji", "reply_to_guid", "thread_originator_guid", "group_action_type", "group_title", "expressive_send_style_id"],
            "chat": ["guid", "service_name", "display_name", "is_archived", "last_read_message_timestamp"],
            "handle": ["id", "service"],
            "chat_message_join": ["chat_id", "message_id"],
            "chat_handle_join": ["chat_id", "handle_id"],
            "attachment": ["guid", "filename", "mime_type", "uti", "total_bytes", "transfer_state"],
            "message_attachment_join": ["message_id", "attachment_id"]
        ]
        let missing = expected.keys.sorted().flatMap { table in expected[table]!.filter { reader?.columns[table]?.contains($0) != true }.map { table + "." + $0 } }
        return .object(["fields": .string(missing.isEmpty ? "complete" : "partial"), "missing": .array(missing.map(JSON.string))])
    }
    @discardableResult private func reopen() throws -> Bool {
        if baseline == nil {
            let saved = try MessagesBaseline(path: path)
            candidates = try saved.identities()
            baseline = saved
            blobDigests = try saved.blobs()
        }
        if let reader, try reader.isCurrent() { return false }
        let fresh = try MessagesReader(path: path)
        let changed = generation != fresh.generation
        if changed {
            generation = fresh.generation
            candidates.formUnion(hashes.keys)
            hashes.removeAll()
            nativeHashes.removeAll()
            hashTimes.removeAll()
            nativeHashTimes.removeAll()
            smallNativeHashes.removeAll()
            firstSmallSnapshot = nil
            blobDigests = try baseline?.blobs() ?? [:]
            pendingBlobs.removeAll()
            failedBlobs.removeAll()
            lastRowID = 0
            lastLiveVersion = nil
            lastComplete = .distantPast
        }
        reader = fresh
        lastLiveVersion = nil
        return changed
    }
    private func observationTime() -> String {
        observationMillisecond = max(Int64(Date().timeIntervalSince1970 * 1000), observationMillisecond + 1)
        return timestamp(Date(timeIntervalSince1970: Double(observationMillisecond) / 1000))
    }
    private func observation(_ collection: String, _ key: String, _ data: [String: Any], _ at: String, kind: String = "upsert") throws -> JSON {
        let raw = try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys])
        let value = try JSONDecoder().decode(JSON.self, from: raw)
        return .object(["kind": .string(kind), "collection": .string(collection), "native_key": .string(key), "data": value, "observed_at": .string(at), "revalidated": .bool(true), "scoped_alias": .string(collection + "/" + key)])
    }
    private func emit(_ items: [(String, String, [String: Any])], sink: any ObservationSink, live: Bool, scanID: String = "", started: String = "", completion: String = "", observedAt: String? = nil) async throws {
        try Task.checkCancellation()
        if items.count > 64 {
            for offset in stride(from: 0, to: items.count, by: 64) {
                let end = min(offset + 64, items.count)
                try await emit(Array(items[offset..<end]), sink: sink, live: live, scanID: scanID, started: started, completion: end == items.count ? completion : "", observedAt: observedAt)
            }
            return
        }
        var observations: [JSON] = []
        var updates: [String: String] = [:]
        var blobHandles: [String: String] = [:]
        let at = observedAt ?? observationTime()
        let outputGeneration = generation
        for (collection, key, original) in items where !key.isEmpty {
            var data = original
            if collection == "messages.attachment", var blob = data["blob"] as? [String: Any], blob["state"] as? String == "present", let reader,
               let row = try reader.query("SELECT * FROM attachment WHERE guid=?", [key]).first, let url = reader.attachmentURL(row["filename"]) {
                do {
                    let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
                    let size = (attributes[.size] as? NSNumber)?.int64Value ?? -1
                    let modified = attributes[.modificationDate] as? Date ?? .distantPast
                    if let cached = blobDigests[key], cached.0 == size, cached.1 == modified {
                        blob["digest"] = cached.2
                        blobHandles[cached.2] = key
                    } else if failedBlobs.contains(key) {
                        blob = ["state": "failed"]
                    } else {
                        blob = ["state": "pending"]
                        pendingBlobs[key] = url
                    }
                    blob.removeValue(forKey: "handle")
                } catch { blob = ["state": "missing"] }
                data["blob"] = blob
            }
            let encoded = try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys])
            let digest = BodyDigest.hash(encoded)
            let identity = collection + "/" + key
            if let newer = hashTimes[identity], newer > at { continue }
            let conflicting = pendingHashes[identity]?.values.contains(where: { $0 != digest }) == true
            if hashes[identity] == digest && !conflicting {
                hashTimes[identity] = at
                continue
            }
            observations.append(try observation(collection, key, data, at))
            updates[identity] = digest
        }
        if observations.isEmpty && completion.isEmpty { return }
        var aliases: [String: JSON] = [:]
        func collect(_ value: JSON) {
            switch value {
            case .object(let fields):
                if let ref = fields["$ref"] {
                    let collection = ref["collection"].string
                    let key = ref["native_key"].string
                    aliases[collection + "/" + key] = .object(["kind": .string("alias"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(collection + "/" + key), "observed_at": .string(at)])
                } else { for field in fields.values { collect(field) } }
            case .array(let values): for child in values { collect(child) }
            default: break
            }
        }
        for item in observations { collect(item["data"]) }
        observations = aliases.keys.sorted().compactMap { aliases[$0] } + observations
        let ticket = UUID()
        for (key, hash) in updates { pendingHashes[key, default: [:]][ticket] = hash }
        defer {
            for key in updates.keys {
                pendingHashes[key]?.removeValue(forKey: ticket)
                if pendingHashes[key]?.isEmpty == true { pendingHashes.removeValue(forKey: key) }
            }
        }
        try baseline?.update(upserts: Array(updates.keys))
        candidates.formUnion(updates.keys)
        try await sink.emit(source: name, generation: outputGeneration, observations: observations, scanID: scanID, scanStartedAt: started, scope: ["messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"], completionEvidence: completion, blobHandles: blobHandles, live: live)
        guard outputGeneration == generation else { throw SourceFailure.corrupt }
        for (key, hash) in updates where (hashTimes[key] ?? "") <= at { hashes[key] = hash; hashTimes[key] = at }
        try Task.checkCancellation()
    }
    private func drainBlobs(_ sink: any ObservationSink) async {
        while !Task.isCancelled {
            guard let (key, url) = pendingBlobs.first else {
                do { try await Task.sleep(for: .milliseconds(50)) } catch { return }
                continue
            }
            pendingBlobs.removeValue(forKey: key)
            let token = generation
            let work = Task.detached { () throws -> (Int64, Date, String) in
                let before = try FileManager.default.attributesOfItem(atPath: url.path)
                let digest = try BlobDigest.file(url)
                let after = try FileManager.default.attributesOfItem(atPath: url.path)
                let size = (before[.size] as? NSNumber)?.int64Value ?? -1
                let modified = before[.modificationDate] as? Date ?? .distantPast
                guard size == (after[.size] as? NSNumber)?.int64Value, modified == after[.modificationDate] as? Date else { throw SourceFailure.conflict }
                return (size, modified, digest)
            }
            do {
                let result = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
                guard token == generation, !Task.isCancelled else { continue }
                try baseline?.saveBlob(key, result)
                blobDigests[key] = result
                if let reader { try await emit(try reader.attachmentObservations(guid: key), sink: sink, live: false) }
            } catch {
                guard token == generation, !Task.isCancelled else { continue }
                failedBlobs.insert(key)
                if let reader { try? await emit(try reader.attachmentObservations(guid: key), sink: sink, live: false) }
            }
        }
    }
    private func emitMessages(_ rows: [SQLiteRow], sink: any ObservationSink, live: Bool, scanID: String = "", started: String = "", observedAt: String? = nil) async throws {
        guard let reader else { throw SourceFailure.unavailable }
        let at = observedAt ?? observationTime()
        let outputGeneration = generation
        let ticket = UUID()
        var items: [(String, String, [String: Any])] = []
        var updates: [String: String] = [:]
        for row in rows {
            var bytes = Data()
            for key in row.values.keys.sorted() {
                let value = row[key]
                bytes.append(contentsOf: "\(key.utf8.count):\(key)\(value.utf8.count):\(value)".utf8)
            }
            for key in row.blobs.keys.sorted() {
                bytes.append(contentsOf: key.utf8)
                bytes.append(contentsOf: BodyDigest.hash(row.blobs[key]!).utf8)
            }
            let hash = BodyDigest.hash(bytes)
            let key = row["guid"]
            if let newer = nativeHashTimes[key], newer > at { continue }
            let conflicting = pendingNativeHashes[key]?.values.contains(where: { $0 != hash }) == true
            if nativeHashes[key] == hash && !conflicting { nativeHashTimes[key] = at; continue }
            items += try reader.messageObservations(row)
            updates[row["guid"]] = hash
        }
        for (key, hash) in updates { pendingNativeHashes[key, default: [:]][ticket] = hash }
        defer {
            for key in updates.keys {
                pendingNativeHashes[key]?.removeValue(forKey: ticket)
                if pendingNativeHashes[key]?.isEmpty == true { pendingNativeHashes.removeValue(forKey: key) }
            }
        }
        try await emit(items, sink: sink, live: live, scanID: scanID, started: started, observedAt: at)
        guard outputGeneration == generation else { throw SourceFailure.corrupt }
        for (key, hash) in updates where (nativeHashTimes[key] ?? "") <= at { nativeHashes[key] = hash; nativeHashTimes[key] = at }
    }
    private func captureInserts(_ sink: any ObservationSink) async throws -> Set<String> {
        guard let reader else { throw SourceFailure.unavailable }
        let maximum = try reader.maximum()
        var cursor = lastRowID
        var referenced: Set<String> = []
        while cursor < maximum {
            let at = observationTime()
            let rows = try reader.messageRows(after: cursor, through: maximum)
            guard !rows.isEmpty else { break }
            for row in rows {
                for field in ["associated_message_guid", "reply_to_guid", "thread_originator_guid"] where !row[field].isEmpty {
                    let value = row[field]
                    if value.hasPrefix("p:"), let slash = value.firstIndex(of: "/") { referenced.insert(String(value[value.index(after: slash)...])) }
                    else { referenced.insert(value.hasPrefix("bp:") ? String(value.dropFirst(3)) : value) }
                }
            }
            try await emitMessages(rows, sink: sink, live: true, observedAt: at)
            cursor = rows.last!.integer("row_position")
            lastRowID = max(lastRowID, cursor)
        }
        return referenced
    }
    private func smallChanges(_ items: [(String, String, [String: Any])], at: String) throws -> [(String, String, [String: Any])] {
        var result: [(String, String, [String: Any])] = []
        for item in items {
            let key = item.0 + "/" + item.1
            let hash = BodyDigest.hash(try JSONSerialization.data(withJSONObject: item.2, options: [.sortedKeys]))
            if let prior = smallNativeHashes[key], prior.1 > at { continue }
            if smallNativeHashes[key]?.0 != hash { result.append(item) }
            smallNativeHashes[key] = (hash, at)
        }
        return result
    }
    private func live(_ sink: any ObservationSink, force: Bool = false) async throws {
        guard let reader else { throw SourceFailure.unavailable }
        let version = try reader.dataVersion()
        if !force && lastLiveVersion == version { return }
        var referenced = try await captureInserts(sink)
        let recentAt = observationTime()
        let recent = try reader.recent()
        for offset in stride(from: 0, to: recent.count, by: 128) {
            try await emitMessages(Array(recent[offset..<min(offset + 128, recent.count)]), sink: sink, live: true, observedAt: recentAt)
            referenced.formUnion(try await captureInserts(sink))
        }
        for guid in referenced {
            if let row = try reader.message(guid) { try await emitMessages([row], sink: sink, live: true) }
        }
        let smallAt = observationTime()
        let changes = try smallChanges(reader.smallObservations(), at: smallAt)
        try await emit(changes, sink: sink, live: true, observedAt: smallAt)
        _ = try await captureInserts(sink)
        lastLiveVersion = version
    }
    private func reconcile(_ sink: any ObservationSink) async throws {
        guard let reader else { throw SourceFailure.unavailable }
        let currentGeneration = generation
        failedBlobs.removeAll()
        let scanID = UUID().uuidString
        let snapshot = firstSmallSnapshot
        firstSmallSnapshot = nil
        let started = snapshot?.1 ?? observationTime()
        let prior = Set(hashes.keys).union(candidates)
        var seen: Set<String> = []
        let small = try snapshot?.0 ?? reader.smallObservations()
        for (collection, key, _) in small { seen.insert(collection + "/" + key) }
        try await emit(small, sink: sink, live: false, scanID: scanID, started: started, observedAt: started)
        let maximum = try reader.maximum()
        var cursor: Int64 = 0
        while cursor < maximum {
            try Task.checkCancellation()
            guard try self.reader?.isCurrent() == true else { throw SourceFailure.corrupt }
            guard generation == currentGeneration, let active = self.reader, try active.isCurrent() else { throw SourceFailure.corrupt }
            let observedAt = observationTime()
            let rows = try active.messageRows(after: cursor, through: maximum)
            guard !rows.isEmpty else { break }
            var freshRows: [SQLiteRow] = []
            for position in rows {
                guard let fresh = try active.message(position["guid"]) else { continue }
                let reaction = MessageReaction.decode(type: Int(fresh.integer("associated_message_type")), target: fresh["associated_message_guid"], emoji: fresh.values["associated_message_emoji"])
                seen.insert((reaction == nil ? "messages.message/" : "messages.reaction/") + fresh["guid"])
                freshRows.append(fresh)
            }
            try await emitMessages(freshRows, sink: sink, live: false, scanID: scanID, started: started, observedAt: observedAt)
            cursor = rows.last!.integer("row_position")
        }
        guard generation == currentGeneration, let active = self.reader, try active.isCurrent() else { throw SourceFailure.corrupt }
        let finalAt = observationTime()
        let finalSmall = try active.smallObservations()
        let smallKeys = Set(finalSmall.map { $0.0 + "/" + $0.1 })
        try await emit(finalSmall, sink: sink, live: false, observedAt: finalAt)
        var absent: [JSON] = []
        var absentKeys: [String] = []
        for key in prior where !seen.contains(key) {
            guard let slash = key.firstIndex(of: "/") else { continue }
            let collection = String(key[..<slash])
            let nativeKey = String(key[key.index(after: slash)...])
            if collection == "messages.message" || collection == "messages.reaction" {
                if try active.message(nativeKey) != nil { continue }
            } else if smallKeys.contains(key) { continue }
            absent.append(try observation(collection, nativeKey, [:], observationTime(), kind: "absent"))
            absentKeys.append(key)
        }
        try await sink.emit(source: name, generation: generation, observations: absent, scanID: scanID, scanStartedAt: started, scope: ["messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"], completionEvidence: "complete", blobHandles: [:], live: false)
        try baseline?.update(absent: absentKeys)
        for key in absentKeys { hashes.removeValue(forKey: key) }
        candidates.removeAll()
        lastComplete = Date()
    }
    private func reportReadFailure(_ error: any Error, _ sink: any ObservationSink) async throws {
        let state = FileManager.default.isReadableFile(atPath: path) ? "degraded" : "permission_missing"
        let reason: String
        if case SourceFailure.corrupt = error { reason = "database_replaced" } else { reason = "chat_db_unavailable" }
        lastComplete = .distantPast
        try await sink.sourceState(source: name, state: state, coverage: .object(["fields": .string("partial"), "missing": .array([.string(reason)])]), capabilities: [], reason: reason)
    }
    private func beginReconciliation(_ sink: any ObservationSink, watcher: MessagesWatcher) {
        if reconciliationTask != nil { reconcileAgain = true; return }
        let id = UUID()
        reconciliationID = id
        reconciliationTask = Task {
            do {
                try await self.reconcile(sink)
                try await sink.sourceState(source: name, state: "ready", coverage: coverage(), capabilities: [], reason: "")
            } catch {
                if !Task.isCancelled { try? await self.reportReadFailure(error, sink) }
            }
            if reconciliationID == id {
                reconciliationID = nil
                reconciliationTask = nil
                if reconcileAgain && !Task.isCancelled { reconcileAgain = false; watcher.requestReconciliation() }
            }
        }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        guard !captureActive else { throw SourceFailure.conflict }
        captureActive = true
        candidates.formUnion(hashes.keys)
        hashes.removeAll()
        nativeHashes.removeAll()
        hashTimes.removeAll()
        nativeHashTimes.removeAll()
        smallNativeHashes.removeAll()
        firstSmallSnapshot = nil
        lastComplete = .distantPast
        lastLiveVersion = nil
        let watcher = MessagesWatcher(path: path, diagnostics: diagnostics)
        var blobs = Task { await self.drainBlobs(sink) }
        var failure: (any Error)?
        var first = true
        do {
            for await wake in watcher.stream {
                try Task.checkCancellation()
                do {
                    let changed = try reopen()
                    if changed {
                        let previous = reconciliationTask
                        previous?.cancel()
                        blobs.cancel()
                        await previous?.value
                        await blobs.value
                        reconciliationTask = nil
                        reconciliationID = nil
                        reconcileAgain = false
                        blobs = Task { await self.drainBlobs(sink) }
                        first = true
                    }
                    if first {
                        lastRowID = try reader?.maximum() ?? 0
                        lastLiveVersion = nil
                        first = false
                        let initialAt = observationTime()
                        let initial = try reader?.smallObservations() ?? []
                        _ = try smallChanges(initial, at: initialAt)
                        firstSmallSnapshot = (initial, initialAt)
                        beginReconciliation(sink, watcher: watcher)
                    } else if wake || watcher.consumeReconciliation() || (reconciliationTask == nil && Date().timeIntervalSince(lastComplete) >= 900) {
                        beginReconciliation(sink, watcher: watcher)
                    }
                    try await live(sink)
                } catch is CancellationError { throw CancellationError() }
                catch {
                    try await reportReadFailure(error, sink)
                    reader = nil
                    first = true
                }
            }
        } catch { failure = error }
        let previous = reconciliationTask
        previous?.cancel()
        blobs.cancel()
        await previous?.value
        await blobs.value
        reconciliationTask = nil
        reconciliationID = nil
        reconcileAgain = false
        watcher.close()
        captureActive = false
        if let failure { throw failure }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { evidence }
    public func readBlob(handle: String, offset: Int64, length: Int) throws -> JSON {
        do {
            let bytes = try readBlobBytes(handle: handle, offset: offset, length: length)
            guard let cached = blobDigests[handle] else { throw SourceFailure.unavailable }
            return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(cached.2), "eof": .bool(offset + Int64(bytes.count) >= cached.0)])
        } catch {
            return .object(["t": .string("BlobUnavailable"), "handle": .string(handle), "reason": .string("missing")])
        }
    }
    private func readBlobBytes(handle: String, offset: Int64, length: Int) throws -> Data {
        guard offset >= 0, length > 0, length <= 128 * 1024, let reader, let row = try reader.query("SELECT * FROM attachment WHERE guid=?", [handle]).first,
              let url = reader.attachmentURL(row["filename"]) else { throw SourceFailure.unavailable }
        let resource = try url.resourceValues(forKeys: [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey])
        guard !(resource.isUbiquitousItem == true && resource.ubiquitousItemDownloadingStatus == .notDownloaded),
              !BlobDigest.isDataless(url), let cached = blobDigests[handle], offset <= cached.0 else { throw SourceFailure.unavailable }
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        try file.seek(toOffset: UInt64(offset))
        return try file.read(upToCount: length) ?? Data()
    }
}
