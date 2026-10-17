import Foundation
import AppKit
import SQLite3
import Darwin
import Decoders
import SourceSupport

@MainActor
public final class UsageSource: Source {
    public nonisolated let name = "usage"
    public static let coverage: JSON = .object(["fields": .string("estimated"), "missing": .array([.string("settings_parity: unproven"), .string("historical sleep without focus lock evidence"), .string("remote batches may lag")])])
    private let root: URL
    private var committed: [String: JSON] = [:]
    private var generation = UUID().uuidString
    private var power: [InFocusEvent] = []
    public init(root: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Biome")) { self.root = root }
    public func open() async -> SourceReadiness {
        let ready = FileManager.default.isReadableFile(atPath: root.appendingPathComponent("streams/restricted/App.InFocus/local").path)
        return SourceReadiness(state: ready ? "ready" : "permission_missing", coverage: Self.coverage, reason: ready ? "" : "full_disk_access_required")
    }
    private func peers() throws -> [(String, String, String, Bool)] {
        var database: OpaquePointer?
        guard sqlite3_open_v2(root.appendingPathComponent("sync/sync.db").path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if let database { sqlite3_close(database) }; throw SourceFailure.unavailable }
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT device_identifier,name,platform,me,model FROM DevicePeer", -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        func text(_ index: Int32) -> String { sqlite3_column_text(statement, index).map { String(cString: $0) } ?? "" }
        var peers: [(String, String, String, Bool)] = []
        var step = sqlite3_step(statement)
        while step == SQLITE_ROW {
            let model = text(4).lowercased(), platform = sqlite3_column_int(statement, 2)
            let kind = platform == 3 ? "mac" : (model.contains("ipad") ? "ipad" : (platform == 2 ? "iphone" : "unknown"))
            peers.append((text(0), text(1), kind, sqlite3_column_int(statement, 3) != 0))
            step = sqlite3_step(statement)
        }
        guard step == SQLITE_DONE else { throw SourceFailure.unavailable }
        return peers
    }
    private func events(_ directory: URL) throws -> [InFocusEvent] {
        var failure = false
        guard let iterator = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: [.isRegularFileKey], options: [.skipsHiddenFiles], errorHandler: { _, _ in failure = true; return false }) else { throw SourceFailure.unavailable }
        var events: [InFocusEvent] = []
        for case let file as URL in iterator {
            if file.pathComponents.contains("tombstone") { iterator.skipDescendants(); continue }
            guard !file.lastPathComponent.isEmpty, file.lastPathComponent.allSatisfy(\.isNumber), try file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else { continue }
            events += InFocus.decode(try Data(contentsOf: file))
        }
        guard !failure else { throw SourceFailure.unavailable }
        return events
    }
    private func scan(_ sink: any ObservationSink, live: Bool) async throws {
        let started = timestamp()
        let base = root.appendingPathComponent("streams/restricted/App.InFocus")
        var current: [String: JSON] = [:]
        let peers = try peers()
        guard peers.contains(where: { $0.3 }) else { throw SourceFailure.unavailable }
        for (identifier, deviceName, platform, local) in peers {
            let directory = local ? base.appendingPathComponent("local") : base.appendingPathComponent("remote/" + identifier)
            guard FileManager.default.fileExists(atPath: directory.path) else { if local { throw SourceFailure.permissionMissing }; continue }
            var input = try events(directory)
            if local { input += power }
            let latest = input.map(\.time).max()
            let key = "usage.device/" + identifier
            current[key] = .object(["name": .string(deviceName), "platform": .string(platform), "build": .string("unavailable"), "is_local": .bool(local), "latest_event_at": latest.map { .string(timestamp(Date(timeIntervalSince1970: $0))) } ?? .null, "provenance": .string("biome.app_infocus"), "decoder_version": .string(InFocus.version)])
            for (key, value) in UsageRollup.records(input, device: identifier, mac: platform == "mac") { current["usage.app_day/" + key] = value }
        }
        var observations: [JSON] = []
        let ordered = current.keys.sorted { a, b in
            let first = a.hasPrefix("usage.device/"), second = b.hasPrefix("usage.device/")
            return first == second ? a < b : first
        }
        for key in ordered where current[key] != committed[key] || !live {
            guard let slash = key.firstIndex(of: "/"), let value = current[key] else { continue }
            let collection = String(key[..<slash]), native = String(key[key.index(after: slash)...])
            observations.append(.object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(native), "scoped_alias": .string("usage/" + key), "data": value, "observed_at": .string(started)]))
        }
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["usage.device", "usage.app_day"], completionEvidence: "complete", blobHandles: [:], live: live)
        committed = current
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        if !resume["generation"].string.isEmpty { generation = resume["generation"].string }
        let (changes, continuation) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let descriptor = Darwin.open(root.appendingPathComponent("streams/restricted/App.InFocus").path, O_EVTONLY)
        guard descriptor >= 0 else { throw SourceFailure.permissionMissing }
        let watcher = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .extend, .rename, .delete], queue: .global())
        watcher.setEventHandler { continuation.yield(()) }
        watcher.setCancelHandler { Darwin.close(descriptor) }; watcher.resume()
        let center = NSWorkspace.shared.notificationCenter
        let observers = [NSWorkspace.willSleepNotification, NSWorkspace.screensDidSleepNotification].map { notification in
            center.addObserver(forName: notification, object: nil, queue: .main) { _ in
                Task { @MainActor in self.power.append(InFocusEvent(time: Date().timeIntervalSince1970, starting: true, bundle: "bridge.power.sleep")); continuation.yield(()) }
            }
        }
        let polling = Task {
            while !Task.isCancelled {
                do { try await Task.sleep(for: .milliseconds(500)); continuation.yield(()) } catch { return }
            }
        }
        defer { polling.cancel(); watcher.cancel(); observers.forEach { center.removeObserver($0) }; continuation.finish() }
        try await scan(sink, live: false)
        for await _ in changes { try Task.checkCancellation(); try await scan(sink, live: true) }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
