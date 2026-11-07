import Foundation
import CryptoKit
@preconcurrency import Photos
import SourceSupport

actor PhotoTransfers {
    private let directory: URL
    private var initialized = false
    private var jobs: [String: PhotoDownload] = [:]
    private var owners: [String: String] = [:]
    private var assets: [String: String] = [:]
    private var lifetimes: [String: TransferLifetime] = [:]
    private var starts: [TransferRequest: (id: String, digest: String)] = [:]
    private var expiryTask: Task<Void, Never>?

    init(home: URL) {
        let identity = SHA256.hash(data: Data(home.standardizedFileURL.path.utf8)).map { String(format: "%02x", $0) }.joined()
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("dev.applebridge.photos-" + identity, isDirectory: true)
        do {
            if FileManager.default.fileExists(atPath: directory.path) { try FileManager.default.removeItem(at: directory) }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            initialized = true
        } catch {
            initialized = false
        }
    }

    func cancelAll() {
        for id in Array(jobs.keys) { remove(id) }
    }

    func fetch(_ args: JSON) throws -> JSON {
        expire()
        try Task.checkCancellation()
        try fields(args, ["asset_id", "resource_index", "resource_key", "network_access", "transfer_id", "offset", "length", "max_bytes", "_principal", "_request"])
        let principal = try string(args, "_principal")
        if args["transfer_id"] != .null {
            guard args["asset_id"] == .null, args["resource_index"] == .null, args["resource_key"] == .null, args["network_access"] == .null, args["max_bytes"] == .null else { throw SourceFailure.invalid }
            let id = try string(args, "transfer_id")
            guard owners[id] == principal, let job = jobs[id], let assetID = assets[id] else { throw SourceFailure.expired }
            do {
                _ = try asset(assetID)
            } catch {
                remove(id)
                throw SourceFailure.expired
            }
            let offset = try integer(args, "offset", default: 0, maximum: 64 << 30)
            let length = try integer(args, "length", default: 128 << 10, maximum: 512 << 10, minimum: 1)
            if var lifetime = lifetimes[id] {
                lifetime.touched = ContinuousClock().now
                lifetimes[id] = lifetime
            }
            do {
                return try job.read(id: id, offset: offset, length: length)
            } catch SourceFailure.invalid {
                throw SourceFailure.invalid
            } catch {
                remove(id)
                throw error
            }
        }
        guard args["offset"] == .null, args["length"] == .null else { throw SourceFailure.invalid }
        let request = TransferRequest(principal: principal, request: try string(args, "_request"))
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let digest = SHA256.hash(data: try encoder.encode(args)).map { String(format: "%02x", $0) }.joined()
        if let started = starts[request] {
            guard started.digest == digest else { throw SourceFailure.invalid }
            return .object(["transfer_id": .string(started.id), "state": .string("downloading"), "idle_expiry_s": .number(120), "max_lifetime_s": .number(86400)])
        }
        guard jobs.count < 4 else { throw SourceFailure.unavailable }
        let photo = try asset(string(args, "asset_id"))
        let resources = PHAssetResource.assetResources(for: photo)
        let original = resources.firstIndex { $0.type == .photo || $0.type == .video || $0.type == .audio }
        guard args["resource_index"] != .null || original != nil else { throw SourceFailure.unsupported }
        let index = try integer(args, "resource_index", default: original ?? 0, maximum: 2_147_483_647)
        guard index < resources.count else { throw SourceFailure.invalid }
        if args["resource_key"] != .null {
            guard try string(args, "resource_key", maximum: 64) == resourceKey(resources[index], asset: photo) else { throw SourceFailure.conflict }
        }
        let network = try boolean(args, "network_access", default: false)
        let maximum = try integer(args, "max_bytes", default: 8 << 30, maximum: 64 << 30, minimum: 1)
        try Task.checkCancellation()
        if !initialized {
            if FileManager.default.fileExists(atPath: directory.path) { try FileManager.default.removeItem(at: directory) }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            initialized = true
        }
        let id = UUID().uuidString
        let job = try PhotoDownload(url: directory.appendingPathComponent(id), maximum: maximum)
        jobs[id] = job
        owners[id] = principal
        assets[id] = photo.localIdentifier
        let now = ContinuousClock().now
        lifetimes[id] = TransferLifetime(started: now, touched: now)
        starts[request] = (id, digest)
        job.start(resources[index], network: network)
        if expiryTask == nil {
            expiryTask = Task {
                while !self.jobs.isEmpty {
                    try? await Task.sleep(for: .seconds(15))
                    self.expire()
                }
                self.expiryTask = nil
            }
        }
        return .object(["transfer_id": .string(id), "state": .string("downloading"), "idle_expiry_s": .number(120), "max_lifetime_s": .number(86400)])
    }

    func release(_ args: JSON) throws -> JSON {
        expire()
        try fields(args, ["transfer_id", "_principal", "_request"])
        let id = try string(args, "transfer_id")
        guard owners[id] == (try string(args, "_principal")) else { throw SourceFailure.expired }
        remove(id)
        return .object(["released": .bool(true)])
    }

    private func remove(_ id: String) {
        jobs.removeValue(forKey: id)?.cancel()
        owners.removeValue(forKey: id)
        assets.removeValue(forKey: id)
        lifetimes.removeValue(forKey: id)
        starts = starts.filter { $0.value.id != id }
    }

    private func expire() {
        let now = ContinuousClock().now
        for (id, lifetime) in lifetimes where lifetime.expired(at: now) {
            remove(id)
        }
    }
}

private struct TransferRequest: Hashable {
    let principal: String
    let request: String
}

struct TransferLifetime {
    let started: ContinuousClock.Instant
    var touched: ContinuousClock.Instant
    func expired(at now: ContinuousClock.Instant) -> Bool {
        touched.duration(to: now) >= .seconds(120) || started.duration(to: now) >= .seconds(86400)
    }
}

final class PhotoDownload: @unchecked Sendable {
    private let lock = NSLock()
    private let url: URL
    private let maximum: Int
    private var file: FileHandle?
    private var request: PHAssetResourceDataRequestID?
    private var hash = SHA256()
    private var size = 0
    private var digest: String?
    private var failed = false
    private var progress = 0.0

    init(url: URL, maximum: Int) throws {
        self.url = url
        self.maximum = maximum
        guard FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw SourceFailure.unavailable }
        do {
            file = try FileHandle(forUpdating: url)
            let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
            guard (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600 else { throw SourceFailure.unavailable }
            try FileManager.default.removeItem(at: url)
        } catch {
            try? file?.close()
            try? FileManager.default.removeItem(at: url)
            throw SourceFailure.unavailable
        }
    }

    deinit { cancel() }

    func start(_ resource: PHAssetResource, network: Bool) {
        let options = PHAssetResourceRequestOptions()
        options.isNetworkAccessAllowed = network
        options.progressHandler = { [weak self] progress in self?.updateProgress(progress) }
        let id = PHAssetResourceManager.default().requestData(for: resource, options: options) { [weak self] data in
            self?.append(data)
        } completionHandler: { [weak self] error in
            self?.finish(failed: error != nil)
        }
        lock.lock()
        request = id
        let stop = failed
        lock.unlock()
        if stop { PHAssetResourceManager.default().cancelDataRequest(id) }
    }

    private func updateProgress(_ value: Double) {
        lock.lock()
        if value.isFinite { progress = max(progress, min(1, max(0, value))) }
        lock.unlock()
    }

    func append(_ data: Data) {
        var cancelID: PHAssetResourceDataRequestID?
        lock.lock()
        defer {
            lock.unlock()
            if let cancelID { PHAssetResourceManager.default().cancelDataRequest(cancelID) }
        }
        guard !failed, digest == nil else { return }
        do {
            guard data.count <= maximum - size, let file else { throw SourceFailure.unavailable }
            if size / (8 << 20) != (size + data.count) / (8 << 20) || size == 0 {
                let attributes = try FileManager.default.attributesOfFileSystem(forPath: url.deletingLastPathComponent().path)
                guard let available = attributes[.systemFreeSize] as? NSNumber, available.int64Value > Int64(data.count) + (2 << 30) else { throw SourceFailure.unavailable }
            }
            try file.write(contentsOf: data)
            hash.update(data: data)
            size += data.count
        } catch {
            failed = true
            try? file?.close()
            file = nil
            try? FileManager.default.removeItem(at: url)
            cancelID = request
        }
    }

    func finish(failed failure: Bool) {
        lock.lock()
        defer { lock.unlock() }
        if failure || size == 0 { failed = true }
        if failed {
            try? file?.close()
            file = nil
            try? FileManager.default.removeItem(at: url)
        } else if digest == nil {
            digest = hash.finalize().map { String(format: "%02x", $0) }.joined()
        }
    }

    func read(id: String, offset: Int, length: Int) throws -> JSON {
        lock.lock()
        defer { lock.unlock() }
        guard !failed else { throw SourceFailure.unavailable }
        guard let digest else {
            return .object(["transfer_id": .string(id), "state": .string("downloading"), "received_bytes": .number(Double(size)), "progress": .number(progress)])
        }
        guard offset <= size else { throw SourceFailure.invalid }
        guard let reader = file else { throw SourceFailure.unavailable }
        try reader.seek(toOffset: UInt64(offset))
        let bytes = try reader.read(upToCount: min(length, size - offset)) ?? Data()
        return .object(["transfer_id": .string(id), "state": .string("ready"), "digest": .string(digest), "size": .number(Double(size)), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "eof": .bool(offset + bytes.count == size)])
    }

    func cancel() {
        lock.lock()
        failed = true
        let id = request
        try? file?.close()
        file = nil
        try? FileManager.default.removeItem(at: url)
        lock.unlock()
        if let id { PHAssetResourceManager.default().cancelDataRequest(id) }
    }
}
