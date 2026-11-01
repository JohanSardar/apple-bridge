import Foundation
import Decoders
import MediaDerivatives
import UniformTypeIdentifiers
import SourceSupport

public actor Messages: Source {
    public nonisolated let name = "messages"
    public static func shouldPreemptMedia(activeDigest: String, incomingDigest: String, activeLive: Bool, incomingImage: Bool) -> Bool {
        activeDigest != incomingDigest && (!activeLive || incomingImage)
    }
    private let path: String
    private let pinsURL: URL?
    private let diagnostics: URL?
    private let workTrace: CaptureTrace
    private let mediaPauseURL: URL?
    private let watcherEventsEnabled: Bool
    private var reader: MessagesReader?
 private let commands: MessagesCommands
    private var baseline: MessagesBaseline?
    private var candidates: Set<String> = []
    private var recoverableGUIDs: Set<String> = []
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
    private var reconciledVersion: Int64?
    private var reconciledMaximum: Int64 = 0
    private var lastLiveVersion: Int64?
    private var lastLivePins: [String]?
    private var lastComplete = Date.distantPast
    private var blobDigests: [String: (Int64, Date, String)] = [:]
    private var pendingBlobs: [String: URL] = [:]
    private var pendingBlobOrder: [String: (Bool, Int64)] = [:]
    private var activeBlob: (String, Bool, Task<(Int64, Date, String), any Error>)?
    private var activeMedia: (MediaJob, Task<MediaResult, any Error>)?
    private var failedBlobs: Set<String> = []
    private struct MediaJob: Sendable {
        let key: String
        let url: URL
        let digest: String
        let companions: [URL]
        let sticker: Bool
        let live: Bool
        let position: Int64
        var queuedNS = CaptureTrace.now()
    }
    private var pendingMedia: [String: MediaJob] = [:]
    private var mediaResults: [String: MediaResult] = [:]
    private var mediaAttempts: [String: (String, Int)] = [:]
    private var mediaRetry: [String: Date] = [:]
    private var mediaPressureChecked = ContinuousClock.now
    private var mediaPressure = false
    private var variantURLs: [String: URL] = [:]
    public init(path: String? = nil, diagnostics: URL? = nil, watcherEventsEnabled: Bool = true, labCommands: Bool = false, pinsURL: URL? = nil) {
        self.pinsURL = pinsURL
 commands = MessagesCommands(path:path, labCommands:labCommands, agentPolicy: diagnostics?.deletingLastPathComponent().appendingPathComponent("agent-message-policy.json"))
        self.watcherEventsEnabled = watcherEventsEnabled
        self.diagnostics = diagnostics
        workTrace = CaptureTrace(url: labCommands ? diagnostics?.deletingLastPathComponent().appendingPathComponent("media-work-trace.jsonl") : nil)
        mediaPauseURL = labCommands ? diagnostics?.deletingLastPathComponent().appendingPathComponent("lab-pause-media") : nil
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
            return SourceReadiness(state: "ready", coverage: await commandCoverage(), capabilities: await commands.capabilities())
        } catch { return SourceReadiness(state: FileManager.default.isReadableFile(atPath: path) ? "degraded" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("chat_db_unavailable")])]), reason: "chat_db_unavailable") }
    }
    private func coverage() -> JSON {
        let expected = [
            "message": ["guid", "date", "text", "attributedBody", "handle_id", "is_from_me", "date_delivered", "date_read", "is_delivered", "is_read", "error", "is_finished", "item_type", "date_edited", "date_retracted", "message_summary_info", "associated_message_guid", "associated_message_type", "associated_message_emoji", "reply_to_guid", "thread_originator_guid", "group_action_type", "group_title", "other_handle", "expressive_send_style_id", "balloon_bundle_id", "payload_data", "is_audio_message", "date_played", "is_expirable", "expire_state", "schedule_type", "schedule_state", "subject", "was_delivered_quietly", "did_notify_recipient", "is_spam", "has_unseen_mention", "is_stewie", "is_sos", "sent_or_received_off_grid"],
            "chat": ["guid", "service_name", "display_name", "is_archived", "last_read_message_timestamp", "is_filtered", "properties"],
            "handle": ["id", "service"],
            "chat_message_join": ["chat_id", "message_id"],
            "chat_handle_join": ["chat_id", "handle_id"],
            "attachment": ["guid", "filename", "mime_type", "uti", "total_bytes", "transfer_state", "is_sticker", "sticker_user_info", "hide_attachment", "emoji_image_content_identifier", "emoji_image_short_description"],
            "message_attachment_join": ["message_id", "attachment_id"],
            "chat_recoverable_message_join": ["chat_id", "message_id", "delete_date"]
        ]
        var missing = ["messages.chat.muted_until", "messages.message.audio.transcription", "messages.message.audio.kept", "messages.message.recoverable_until", "messages.chat.recoverable_until"]
        if MessagesParity.muteProfile() == .null { missing.append("messages.chat.muted") }
        if (try? MessagesPins.read(at: pinsURL)) == nil { missing.append("messages.chat.pinned_index") }
        missing += expected.keys.sorted().flatMap { table in expected[table]!.filter { reader?.columns[table]?.contains($0) != true }.map { table + "." + $0 } }
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
        let fresh = try MessagesReader(path: path, pinsURL: pinsURL)
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
            pendingBlobOrder.removeAll()
            activeBlob?.2.cancel()
            pendingMedia.removeAll()
            mediaAttempts.removeAll()
            activeMedia?.1.cancel()
            failedBlobs.removeAll()
            lastRowID = 0
            lastLiveVersion = nil
            lastComplete = .distantPast
        }
        reconciledVersion = nil
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
    private func emit(_ items: [(String, String, [String: Any])], sink: any ObservationSink, live: Bool, scanID: String = "", started: String = "", completion: String = "", observedAt: String? = nil, force: Bool = false) async throws {
        try Task.checkCancellation()
        if items.count > 64 {
            for offset in stride(from: 0, to: items.count, by: 64) {
                let end = min(offset + 64, items.count)
                try await emit(Array(items[offset..<end]), sink: sink, live: live, scanID: scanID, started: started, completion: end == items.count ? completion : "", observedAt: observedAt, force: force)
            }
            return
        }
        var observations: [JSON] = []
        var updates: [String: String] = [:]
        var blobHandles: [String: String] = [:]
        let at = observedAt ?? observationTime()
        let outputGeneration = generation
        workTrace.record("attachment_prepare_compute_started")
        for (collection, key, original) in items where !key.isEmpty {
            var data = original
            if collection == "messages.attachment", var blob = data["blob"] as? [String: Any], blob["state"] as? String == "present", let reader,
               let row = try reader.query("SELECT ROWID AS derivative_position,* FROM attachment WHERE guid=?", [key]).first, let url = reader.attachmentURL(row["filename"]) {
                do {
                    let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
                    let size = (attributes[.size] as? NSNumber)?.int64Value ?? -1
                    let modified = attributes[.modificationDate] as? Date ?? .distantPast
                    if let cached = blobDigests[key], cached.0 == size, cached.1 == modified {
                        blob["digest"] = cached.2
                        blobHandles[cached.2] = key
                        if MediaDerivatives.supports(url, uti: row["uti"], mime: data["mime_type"] as? String ?? row["mime_type"]) {
                            let result = mediaResults[cached.2] ?? MediaDerivatives.cached(cached.2)
                            data["media"] = [:] as [String: Any]
                            if result == nil && mediaRetry[key] != nil { data["derivatives"] = [:] as [String: Any] }
                            if let result {
                                mediaResults[cached.2] = result
                                try attachMedia(result, to: &data, handles: &blobHandles)
                            }
                            if !mediaLabPaused, (result == nil || result?.awaitingLive == true || result.map(MediaDerivatives.needsPosterUpgrade) == true), activeMedia?.0.key != key, (mediaRetry[key] ?? .distantPast) < Date() {
                                let companions = MediaDerivatives.liveIdentifier(url) == nil ? [] : try reader.query("SELECT filename FROM attachment WHERE lower(filename) LIKE '%.mov' ORDER BY abs(ROWID-?)", [String(row.integer("derivative_position"))]).compactMap { reader.attachmentURL($0["filename"]) }.filter { FileManager.default.fileExists(atPath: $0.path) && !BlobDigest.isDataless($0) }
                                var job = MediaJob(key: key, url: url, digest: cached.2, companions: companions, sticker: row.flag("is_sticker"), live: live, position: row.integer("derivative_position"))
                                if let pending = pendingMedia[key], pending.digest == job.digest { job.queuedNS = pending.queuedNS }
                                if pendingMedia[key]?.live != true {
                                    pendingMedia[key] = job
                                    workTrace.record("media_queued", fields: ["pending_media": .number(Double(pendingMedia.count))])
                                }
                                if live, let activeMedia, Self.shouldPreemptMedia(activeDigest: activeMedia.0.digest, incomingDigest: job.digest, activeLive: activeMedia.0.live, incomingImage: (UTType(filenameExtension: url.pathExtension) ?? UTType(row["uti"]))?.conforms(to: .image) == true) {
                                    workTrace.record("media_preempted_by_live")
                                    activeMedia.1.cancel()
                                }

                            }
                        }
                    } else if failedBlobs.contains(key) {
                        blob = ["state": "failed"]
                    } else {
                        blob = ["state": "pending"]
                        pendingBlobs[key] = url
                        pendingBlobOrder[key] = (live, row.integer("derivative_position"))
                        if live, let activeBlob, activeBlob.0 != key, !activeBlob.1 || (UTType(filenameExtension: url.pathExtension) ?? UTType(row["uti"]))?.conforms(to: .image) == true { activeBlob.2.cancel() }
                    }
                    blob.removeValue(forKey: "handle")
                } catch { blob = ["state": "missing"] }
                data["blob"] = blob
            } else if collection == "messages.attachment", var blob = data["blob"] as? [String: Any], ["offloaded", "missing"].contains(blob["state"] as? String ?? ""), let cached = blobDigests[key], let result = mediaResults[cached.2] ?? MediaDerivatives.cached(cached.2) {
                blob["digest"] = cached.2
                blob.removeValue(forKey: "handle")
                data["blob"] = blob
                try attachMedia(result, to: &data, handles: &blobHandles)
            }
            let encoded = try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys])
            let digest = BodyDigest.hash(encoded)
            let identity = collection + "/" + key
            if let newer = hashTimes[identity], newer > at { continue }
            let conflicting = pendingHashes[identity]?.values.contains(where: { $0 != digest }) == true
            if !force && hashes[identity] == digest && !conflicting {
                hashTimes[identity] = at
                continue
            }
            observations.append(try observation(collection, key, data, at))
            updates[identity] = digest
        }
        workTrace.record("attachment_prepare_compute_finished")
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
    private func attachMedia(_ result: MediaResult, to data: inout [String: Any], handles: inout [String: String]) throws {
        data["media"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(result.media))
        var derivatives: [String: Any] = [:]
        for (name, variant) in result.derivatives {
            var value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(variant)) as? [String: Any] ?? [:]
            value.removeValue(forKey: "file")
            value["state"] = "present"
            derivatives[name] = value
            handles[variant.digest] = variant.handle
            variantURLs[variant.handle] = MediaDerivatives.url(variant)
            blobDigests[variant.handle] = (variant.size, .distantPast, variant.digest)
        }
        for (name, reason) in result.unavailable ?? [:] { derivatives[name] = ["state": "unavailable", "reason": reason] }
        data["derivatives"] = derivatives
    }
    private func publishMedia(_ result: MediaResult, job: MediaJob, token: String, sink: any ObservationSink) async throws {
        guard token == generation, blobDigests[job.key]?.2 == job.digest, let reader else { return }
        mediaResults[job.digest] = result
        try await emit(try reader.attachmentObservations(guid: job.key), sink: sink, live: job.live)
    }
    private var mediaLabPaused: Bool {
        guard let mediaPauseURL else { return false }
        return FileManager.default.fileExists(atPath: mediaPauseURL.path)
    }
    private var mediaPressurePaused: Bool {
        if mediaPressureChecked <= ContinuousClock.now {
            mediaPressure = MediaDerivatives.paused
            mediaPressureChecked = ContinuousClock.now + .seconds(1)
        }
        return mediaPressure || mediaLabPaused
    }
    private func mediaCheckpoint(_ job: MediaJob, token: String, stamp: MediaSourceStamp, quiet: Duration) async throws {
        try await ForegroundActivity.shared.waitUntilIdle(for: quiet)
        while mediaPressurePaused {
            try await Task.sleep(for: .milliseconds(100))
            try await ForegroundActivity.shared.waitUntilIdle(for: quiet)
        }
        try Task.checkCancellation()
        guard token == generation, blobDigests[job.key]?.2 == job.digest else { throw MediaSourceChanged() }
        try stamp.validate(job.url)
        let attributes = try FileManager.default.attributesOfItem(atPath: job.url.path)
        guard let source = blobDigests[job.key], source.0 == (attributes[.size] as? NSNumber)?.int64Value, source.1 == attributes[.modificationDate] as? Date else { throw MediaSourceChanged() }
    }
    private func drainMedia(_ sink: any ObservationSink) async {
        var lastPaused: Bool?
        while !Task.isCancelled {
            guard pendingMedia.values.contains(where: { (mediaRetry[$0.key] ?? .distantPast) < Date() }) else {
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                continue
            }
            do { try await ForegroundActivity.shared.waitUntilIdle(for: .milliseconds(250)) } catch { return }
            if Task.isCancelled { return }
            let paused = mediaPressurePaused
            let labPaused = mediaLabPaused
            let systemPaused = mediaPressure
            if paused != lastPaused {
                workTrace.record(paused ? "media_paused" : "media_resumed", fields: ["lab_paused": .bool(labPaused), "system_paused": .bool(systemPaused), "thermal": .number(Double(ProcessInfo.processInfo.thermalState.rawValue))])
                lastPaused = paused
            }
            guard !paused, let job = pendingMedia.values.filter({ (mediaRetry[$0.key] ?? .distantPast) < Date() }).sorted(by: { a, b in a.live == b.live ? a.position > b.position : a.live }).first else {
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                continue
            }
            let previous = mediaAttempts[job.key]
            let attempts = previous?.0 == job.digest ? previous!.1 + 1 : 1
            if attempts > 3 {
                do { try await ForegroundActivity.shared.waitUntilIdle(for: .seconds(2)) } catch { return }
            }
            mediaAttempts[job.key] = (job.digest, attempts)
            pendingMedia.removeValue(forKey: job.key)
            let token = generation
            workTrace.record("media_started", keys: [job.digest], fields: ["queued_ns": .integer(job.queuedNS), "pending_media": .number(Double(pendingMedia.count)), "live": .bool(job.live)])
            let workTrace = self.workTrace
            let work = Task.detached(executorPreference: MediaExecutor.shared, priority: .utility) {
                try Task.checkCancellation()
                workTrace.record("media_compute_started", keys: [job.digest], fields: ["queued_ns": .integer(job.queuedNS)])
                defer { workTrace.record("media_compute_finished") }
                let stamp = try MediaSourceStamp(job.url, digest: job.digest)
                let activity = MediaWork(digest: job.digest, checkpoint: { quiet in
                    try await self.mediaCheckpoint(job, token: token, stamp: stamp, quiet: quiet)
                }, interruption: { callback in
                    let id = ForegroundActivity.shared.interrupt(callback)
                    let pressure = Task {
                        while !Task.isCancelled {
                            do { try await Task.sleep(for: .seconds(1)) } catch { return }
                            if await self.mediaPressurePaused { callback(); return }
                        }
                    }
                    return { ForegroundActivity.shared.removeInterruption(id); pressure.cancel() }
                }, trace: { stage in workTrace.record("media_stage_" + stage, fields: ["queued_ns": .integer(job.queuedNS)]) })
                return try await MediaWork.$current.withValue(activity) {
                    try await MediaDerivatives.make(job.url, digest: job.digest, companions: job.companions, sticker: job.sticker) { result in
                        try stamp.validate(job.url)
                        try await self.publishMedia(result, job: job, token: token, sink: sink)
                    }
                }
            }
            activeMedia = (job, work)
            let interruption = ForegroundActivity.shared.interrupt {
                workTrace.record("media_yielded_to_foreground")
            }
            defer { ForegroundActivity.shared.removeInterruption(interruption) }
            defer { activeMedia = nil }
            do {
                let result = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
                guard token == generation, blobDigests[job.key]?.2 == job.digest, !Task.isCancelled else { continue }
                try await publishMedia(result, job: job, token: token, sink: sink)
                if result.awaitingLive == true {
                    workTrace.record("media_waiting_for_companion", fields: ["queued_ns": .integer(job.queuedNS)])
                    mediaRetry[job.key] = Date().addingTimeInterval(60)
                    if attempts >= 3 { workTrace.record("media_idle_deferred", fields: ["queued_ns": .integer(job.queuedNS)]) }
                    if let reader {
                        let companions = try reader.query("SELECT filename FROM attachment WHERE lower(filename) LIKE '%.mov' ORDER BY abs(ROWID-?)", [String(job.position)]).compactMap { reader.attachmentURL($0["filename"]) }.filter { FileManager.default.fileExists(atPath: $0.path) && !BlobDigest.isDataless($0) }
                        var deferred = MediaJob(key: job.key, url: job.url, digest: job.digest, companions: companions, sticker: job.sticker, live: false, position: job.position)
                        deferred.queuedNS = job.queuedNS
                        pendingMedia[job.key] = deferred
                    }
                } else {
                    workTrace.record("media_completed", fields: ["queued_ns": .integer(job.queuedNS), "unavailable": .bool(result.unavailable != nil)])
                    mediaAttempts.removeValue(forKey: job.key)
                }
            } catch {
                guard token == generation, !Task.isCancelled else { continue }
                if error is MediaSourceChanged {
                    workTrace.record("media_source_changed", fields: ["queued_ns": .integer(job.queuedNS)])
                    mediaResults.removeValue(forKey: job.digest)
                    mediaAttempts.removeValue(forKey: job.key)
                    if let reader, let row = try? reader.query("SELECT filename FROM attachment WHERE guid=?", [job.key]).first, let url = reader.attachmentURL(row["filename"]) {
                        failedBlobs.remove(job.key)
                        pendingBlobs[job.key] = url
                        pendingBlobOrder[job.key] = (job.live, job.position)
                    }
                } else if work.isCancelled && !Task.isCancelled {
                    pendingMedia[job.key] = job
                    if attempts >= 3 {
                        mediaRetry[job.key] = Date().addingTimeInterval(60)
                        workTrace.record("media_idle_deferred", fields: ["queued_ns": .integer(job.queuedNS)])
                    }
                }
                else {
                    mediaRetry[job.key] = Date().addingTimeInterval(60)
                    if let reader { try? await emit(try reader.attachmentObservations(guid: job.key), sink: sink, live: job.live) }
                }
            }
        }
    }
    private func drainBlobs(_ sink: any ObservationSink) async {
        while !Task.isCancelled {
            let next = pendingBlobs.keys.sorted { a, b in
                let x = pendingBlobOrder[a] ?? (false, 0)
                let y = pendingBlobOrder[b] ?? (false, 0)
                return x.0 == y.0 ? x.1 > y.1 : x.0
            }.first
            guard let key = next, let url = pendingBlobs[key] else {
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                continue
            }
            pendingBlobs.removeValue(forKey: key)
            let order = pendingBlobOrder.removeValue(forKey: key) ?? (false, 0)
            let live = order.0
            let token = generation
            workTrace.record("hash_started", fields: ["pending_blobs": .number(Double(pendingBlobs.count)), "live": .bool(live)])
            let workTrace = self.workTrace
            let work = Task.detached(priority: .utility) { () throws -> (Int64, Date, String) in
                workTrace.record("hash_compute_started")
                defer { workTrace.record("hash_compute_finished") }
                let stamp = try MediaSourceStamp(url, digest: "")
                let before = try FileManager.default.attributesOfItem(atPath: url.path)
                let digest = try await BlobDigest.background(url)
                try stamp.validate(url)
                let after = try FileManager.default.attributesOfItem(atPath: url.path)
                let size = (before[.size] as? NSNumber)?.int64Value ?? -1
                let modified = before[.modificationDate] as? Date ?? .distantPast
                guard size == (after[.size] as? NSNumber)?.int64Value, modified == after[.modificationDate] as? Date else { throw MediaSourceChanged() }
                return (size, modified, digest)
            }
            activeBlob = (key, live, work)
            defer { activeBlob = nil }
            do {
                let result = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
                guard token == generation, !Task.isCancelled else { continue }
                try baseline?.saveBlob(key, result)
                blobDigests[key] = result
                if UTType(filenameExtension: url.pathExtension)?.conforms(to: .movie) == true, let reader {
                    for (mediaKey, job) in pendingMedia where mediaResults[job.digest]?.awaitingLive == true {
                        let companions = try reader.query("SELECT filename FROM attachment WHERE lower(filename) LIKE '%.mov' ORDER BY abs(ROWID-?)", [String(job.position)]).compactMap { reader.attachmentURL($0["filename"]) }.filter { FileManager.default.fileExists(atPath: $0.path) && !BlobDigest.isDataless($0) }
                        var resumed = MediaJob(key: job.key, url: job.url, digest: job.digest, companions: companions, sticker: job.sticker, live: job.live, position: job.position)
                        resumed.queuedNS = job.queuedNS
                        pendingMedia[mediaKey] = resumed
                        mediaRetry[mediaKey] = .distantPast
                    }
                }
                if let reader { try await emit(try reader.attachmentObservations(guid: key), sink: sink, live: live) }
            } catch {
                guard token == generation, !Task.isCancelled else { continue }
                if work.isCancelled || error is MediaSourceChanged {
                    if pendingBlobs[key] == nil { pendingBlobs[key] = url; pendingBlobOrder[key] = order }
                    continue
                }
                failedBlobs.insert(key)
                if let reader { try? await emit(try reader.attachmentObservations(guid: key), sink: sink, live: false) }
            }
        }
    }
    private func emitMessages(_ rows: [SQLiteRow], sink: any ObservationSink, live: Bool, scanID: String = "", started: String = "", observedAt: String? = nil, force: Bool = false) async throws {
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
            if !force && nativeHashes[key] == hash && !conflicting { nativeHashTimes[key] = at; continue }
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
        try await emit(items, sink: sink, live: live, scanID: scanID, started: started, observedAt: at, force: force)
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
        let pins = try? MessagesPins.read(at: pinsURL)
        if !force && lastLiveVersion == version {
            if lastLivePins != pins {
                let at = observationTime()
                try await emit(try smallChanges(reader.smallObservations(), at: at), sink: sink, live: true, observedAt: at)
                lastLivePins = pins
            }
            return
        }
        var referenced = try await captureInserts(sink)
        let recoverable = try reader.recoverableGUIDs()
        referenced.formUnion(recoverable.union(recoverableGUIDs))
        recoverableGUIDs = recoverable
        let recentAt = observationTime()
        let recent = try reader.recent()
        for offset in stride(from: 0, to: recent.count, by: 128) {
            try await emitMessages(Array(recent[offset..<min(offset + 128, recent.count)]), sink: sink, live: true, observedAt: recentAt)
            referenced.formUnion(try await captureInserts(sink))
        }
        let smallAt = observationTime()
        let changes = try smallChanges(reader.smallObservations(), at: smallAt)
        for (collection, _, data) in changes where collection == "messages.attachment" {
            if let reference = (data["message_id"] as? [String: Any])?["$ref"] as? [String: String], let key = reference["native_key"] { referenced.insert(key) }
        }
        try await emit(changes, sink: sink, live: true, observedAt: smallAt)
        referenced.formUnion(try await captureInserts(sink))
        while true {
            let keys = referenced.sorted()
            for offset in stride(from: 0, to: keys.count, by: 128) {
                let rows = try reader.messages(Array(keys[offset..<min(offset + 128, keys.count)]))
                try await emitMessages(rows, sink: sink, live: true)
            }
            let position = lastRowID
            referenced = try await captureInserts(sink)
            if referenced.isEmpty && position == lastRowID { break }
        }
        lastLiveVersion = version
        lastLivePins = pins
    }
    private func reconcile(_ sink: any ObservationSink) async throws {
        guard let reader else { throw SourceFailure.unavailable }
        let currentGeneration = generation
        let version = try reader.dataVersion()
        let maximum = try reader.maximum()
        if reconciledVersion == version && reconciledMaximum == maximum {
            lastComplete = Date()
            workTrace.record("messages_reconcile_unchanged")
            return
        }
        workTrace.record("messages_reconcile_started")
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
        var cursor: Int64 = 0
        while cursor < maximum {
            try Task.checkCancellation()
            guard try self.reader?.isCurrent() == true else { throw SourceFailure.corrupt }
            guard generation == currentGeneration, let active = self.reader, try active.isCurrent() else { throw SourceFailure.corrupt }
            let observedAt = observationTime()
            let rows = try active.messageRows(after: cursor, through: maximum)
            guard !rows.isEmpty else { break }
            for row in rows {
                let reaction = MessageReaction.decode(type: Int(row.integer("associated_message_type")), target: row["associated_message_guid"], emoji: row.values["associated_message_emoji"])
                seen.insert((reaction == nil ? "messages.message/" : "messages.reaction/") + row["guid"])
            }
            try await emitMessages(rows, sink: sink, live: false, scanID: scanID, started: started, observedAt: observedAt)
            cursor = rows.last!.integer("row_position")
            await Task.yield()
        }
        guard generation == currentGeneration, let active = self.reader, try active.isCurrent() else { throw SourceFailure.corrupt }
        let finalAt = observationTime()
        let finalSmall = try active.smallObservations()
        let smallKeys = Set(finalSmall.map { $0.0 + "/" + $0.1 })
        try await emit(finalSmall, sink: sink, live: false, observedAt: finalAt)
        var absent: [JSON] = []
        var absentKeys: [String] = []
        let missing = prior.subtracting(seen)
        let messageKeys = missing.filter { $0.hasPrefix("messages.message/") || $0.hasPrefix("messages.reaction/") }.map { String($0.split(separator: "/", maxSplits: 1)[1]) }
        var retained: Set<String> = []
        for offset in stride(from: 0, to: messageKeys.count, by: 128) {
            retained.formUnion(try active.existingMessages(Array(messageKeys[offset..<min(offset + 128, messageKeys.count)])))
        }
        for key in missing {
            guard let slash = key.firstIndex(of: "/") else { continue }
            let collection = String(key[..<slash])
            let nativeKey = String(key[key.index(after: slash)...])
            if collection == "messages.message" || collection == "messages.reaction" {
                if retained.contains(nativeKey) { continue }
            } else {
                let table = collection == "messages.chat" ? "chat" : collection == "messages.handle" ? "handle" : "attachment"
                let required = table == "handle" ? ["id", "service"] : ["guid"]
                if !required.allSatisfy({ active.columns[table]?.contains($0) == true }) || smallKeys.contains(key) { continue }
            }
            absent.append(try observation(collection, nativeKey, [:], observationTime(), kind: "absent"))
            absentKeys.append(key)
        }
        try await sink.emit(source: name, generation: generation, observations: absent, scanID: scanID, scanStartedAt: started, scope: ["messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"], completionEvidence: "complete", blobHandles: [:], live: false)
        try baseline?.update(absent: absentKeys)
        for key in absentKeys { hashes.removeValue(forKey: key) }
        candidates.removeAll()
        if self.reader === reader {
            reconciledVersion = version
            reconciledMaximum = maximum
        }
        workTrace.record("messages_reconcile_completed")
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
        reconciledVersion = nil
        lastLiveVersion = nil
        let watcher = MessagesWatcher(path: path, diagnostics: diagnostics, eventsEnabled: watcherEventsEnabled)
        var blobs = Task { await self.drainBlobs(sink) }
        let media = Task { await self.drainMedia(sink) }
        defer { media.cancel() }
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
                    let requested = watcher.consumeReconciliation()
                    if first {
                        lastRowID = try reader?.maximum() ?? 0
                        lastLiveVersion = nil
                        first = false
                        let initialAt = observationTime()
                        let initial = try reader?.smallObservations() ?? []
                        _ = try smallChanges(initial, at: initialAt)
                        firstSmallSnapshot = (initial, initialAt)
                        beginReconciliation(sink, watcher: watcher)
                    } else if wake || requested || (reconciliationTask == nil && Date().timeIntervalSince(lastComplete) >= 900) {
                        workTrace.record("messages_reconcile_requested", fields: ["wake": .bool(wake), "requested": .bool(requested), "active": .bool(reconciliationTask != nil)])
                        beginReconciliation(sink, watcher: watcher)
                    }
                    try await live(sink)
                    if let diagnostics {
                        let readyMedia = pendingMedia.keys.contains { (mediaRetry[$0] ?? .distantPast) < Date() }
                        let idle = reconciliationTask == nil && lastComplete != .distantPast && pendingBlobs.isEmpty && activeBlob == nil && !readyMedia && activeMedia == nil
                        let data = try JSONSerialization.data(withJSONObject: ["idle": idle])
                        try data.write(to: diagnostics.deletingLastPathComponent().appendingPathComponent("messages-idle.json"), options: .atomic)
                    }
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
        media.cancel()
        await media.value
        await previous?.value
        await blobs.value
        reconciliationTask = nil
        reconciliationID = nil
        reconcileAgain = false
        watcher.close()
        captureActive = false
        if let failure { throw failure }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { try await commands.prepare(command) }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { try await commands.dispatch(prepared) }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { await commands.resolve(evidence) }
    private func commandCoverage() async -> JSON {
        var value = coverage().object
        value["operations"] = await commands.operationCoverage()
        return .object(value)
    }
    public func captureCommand(_ evidence: JSON, into sink: any ObservationSink) async throws {
        _ = try reopen()
        guard let reader else {throw SourceFailure.unavailable}
        if evidence["collection"].string == "messages.chat" {
            let key = evidence["native_key"].string
            let rows = try reader.smallObservations().filter { $0.0 == "messages.chat" && $0.1 == key }
            if !rows.isEmpty { try await emit(rows, sink: sink, live: true, force: true) }
            else {
                guard evidence["absent"].bool, try reader.query("SELECT guid FROM chat WHERE guid=?", [key]).isEmpty else { throw SourceFailure.conflict }
                let at = observationTime()
                let absent = try observation("messages.chat", key, [:], at, kind: "absent")
                try await sink.emit(source: name, generation: generation, observations: [absent], scanID: UUID().uuidString, scanStartedAt: at, scope: ["messages.chat"], completionEvidence: "complete", blobHandles: [:], live: true)
            }
            return
        }
        var keys = Set(evidence["parts"].array.map { $0["native_key"].string }.filter { !$0.isEmpty })
        if !evidence["native_key"].string.isEmpty { keys.insert(evidence["native_key"].string) }
        let rows = try keys.sorted().compactMap { try reader.message($0) }
        if !rows.isEmpty { try await emitMessages(rows, sink: sink, live: true, force: true) }
        if try reader.message(evidence["native_key"].string) == nil, evidence["absent"].bool {
            let at=observationTime()
            let absent=try observation("messages.message",evidence["native_key"].string,[:],at,kind:"absent")
            try await sink.emit(source:name,generation:generation,observations:[absent],scanID:UUID().uuidString,scanStartedAt:at,scope:["messages.message"],completionEvidence:"complete",blobHandles:[:],live:true)
        }
        for part in evidence["parts"].array {
            let attachments = try reader.query("SELECT a.guid FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?",[part["native_key"].string])
            for attachment in attachments {try await emit(try reader.attachmentObservations(guid:attachment["guid"]),sink:sink,live:true)}
        }
    }
    public func validate(_ prepared: Prepared) async throws {try await commands.validate(prepared)}
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
        if let url = variantURLs[handle], let cached = blobDigests[handle] {
            guard offset >= 0, offset <= cached.0, length > 0, length <= 128 * 1024 else { throw SourceFailure.invalid }
            let file = try FileHandle(forReadingFrom: url)
            defer { try? file.close() }
            try file.seek(toOffset: UInt64(offset))
            return try file.read(upToCount: length) ?? Data()
        }
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
