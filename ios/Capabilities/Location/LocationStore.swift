import Foundation

struct LocationFix: Codable, Equatable {
    let latitude: Double
    let longitude: Double
    let accuracy: Double
    let timestamp: Date
}

struct NamedGeofence: Codable, Equatable {
    let name: String
    let latitude: Double
    let longitude: Double
    let radius: Double
}

struct LocationRecord: Codable {
    let id: String
    let kind: String
    let name: String?
    let fix: LocationFix?
    let timestamp: Date
    var fence: NamedGeofence? = nil
}

struct LocationSnapshot: Codable {
    var fences: [NamedGeofence] = []
    var significant = false
    var records: [LocationRecord] = []
}

enum LocationFailure: Error {
    case permission, invalid, unavailable, busy
}

@MainActor
protocol LocationAdapter: AnyObject {
    var onEvent: ((String, String?, LocationFix?) -> Void)? { get set }
    func current() async throws -> LocationFix
    func monitor(_ fence: NamedGeofence) async throws
    func remove(_ name: String)
    func significant(_ enabled: Bool) throws
}

@MainActor
final class LocationStore {
    let adapter: any LocationAdapter
    let url: URL
    private(set) var snapshot: LocationSnapshot
    var changed: (() -> Void)?
    private(set) var persistenceFailed = false

    init(adapter: any LocationAdapter, url: URL) throws {
        self.adapter = adapter
        self.url = url
        snapshot = FileManager.default.fileExists(atPath: url.path) ? try JSONDecoder().decode(LocationSnapshot.self, from: Data(contentsOf: url)) : LocationSnapshot()
        adapter.onEvent = { [weak self] kind, name, fix in
            guard let self else { return }
            do { try self.record(kind, name: name, fix: fix); self.changed?() }
            catch { self.persistenceFailed = true }
        }
    }

    func save() throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        #if os(iOS)
        try JSONEncoder().encode(snapshot).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try JSONEncoder().encode(snapshot).write(to: url, options: .atomic)
        #endif
    }

    func restore() async throws {
        if snapshot.significant { try adapter.significant(true) }
        for fence in snapshot.fences { try await adapter.monitor(fence) }
    }

    func record(_ kind: String, name: String? = nil, fix: LocationFix? = nil, fence: NamedGeofence? = nil) throws {
        guard ["current", "arrive", "leave", "significant", "geofence", "geofence_removed"].contains(kind), kind == "geofence_removed" || name == nil || snapshot.fences.contains(where: { $0.name == name }) else { throw LocationFailure.invalid }
        snapshot.records.append(LocationRecord(id: UUID().uuidString, kind: kind, name: name, fix: fix, timestamp: Date(), fence: fence))
        try save()
    }

    func current() async throws {
        let fix = try await adapter.current()
        guard fix.latitude.isFinite, fix.longitude.isFinite, abs(fix.latitude) <= 90, abs(fix.longitude) <= 180, fix.accuracy >= 0, abs(fix.timestamp.timeIntervalSinceNow) <= 30 else { throw LocationFailure.unavailable }
        try record("current", fix: fix)
    }

    func add(_ fence: NamedGeofence) async throws {
        guard !fence.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, fence.name.utf8.count <= 100, fence.latitude.isFinite, fence.longitude.isFinite, fence.radius.isFinite, abs(fence.latitude) <= 90, abs(fence.longitude) <= 180, fence.radius >= 1, fence.radius <= 100000, snapshot.fences.count < 20 || snapshot.fences.contains(where: { $0.name == fence.name }) else { throw LocationFailure.invalid }
        try await adapter.monitor(fence)
        snapshot.fences.removeAll { $0.name == fence.name }
        snapshot.fences.append(fence)
        try record("geofence", name: fence.name, fence: fence)
    }

    func remove(_ name: String) throws {
        adapter.remove(name)
        snapshot.fences.removeAll { $0.name == name }
        try record("geofence_removed", name: name)
    }

    func significant(_ enabled: Bool) throws {
        try adapter.significant(enabled)
        snapshot.significant = enabled
        try save()
    }

    func acknowledge(_ ids: Set<String>) throws {
        snapshot.records.removeAll { ids.contains($0.id) }
        try save()
    }
}

@MainActor
enum LocationQualification {
    static func remove(_ name: String, timeout: TimeInterval = 20, remove: (String) -> Void, isMonitoring: (String) -> Bool) async -> Bool {
        remove(name)
        return await QualificationWait.until(timeout: timeout) { !isMonitoring(name) }
    }
}
