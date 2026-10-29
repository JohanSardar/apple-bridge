import Foundation

@MainActor
final class FakeLocation: LocationAdapter {
    var onEvent: ((String, String?, LocationFix?) -> Void)?
    var allowed = true
    var registrationFails = false
    var fences: [String: NamedGeofence] = [:]
    var significantEnabled = false
    let fix = LocationFix(latitude: 12, longitude: 34, accuracy: 3, timestamp: Date())
    func current() async throws -> LocationFix {
        guard allowed else { throw LocationFailure.permission }
        return fix
    }
    func monitor(_ fence: NamedGeofence) async throws {
        guard allowed else { throw LocationFailure.permission }
        await Task.yield()
        guard !registrationFails else { throw LocationFailure.unavailable }
        fences[fence.name] = fence
    }
    func remove(_ name: String) { fences.removeValue(forKey: name) }
    func significant(_ enabled: Bool) throws {
        guard allowed || !enabled else { throw LocationFailure.permission }
        significantEnabled = enabled
    }
}

@main
struct LocationProof {
    @MainActor
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("records.json")
        let fake = FakeLocation()
        let store = try LocationStore(adapter: fake, url: url)
        try await store.current()
        guard store.snapshot.records.last?.fix == fake.fix else { throw LocationFailure.invalid }
        let fence = NamedGeofence(name: "[ab-test] fence", latitude: 12, longitude: 34, radius: 150)
        try await store.add(fence)
        guard store.snapshot.fences == [fence], fake.fences.count == 1 else { throw LocationFailure.invalid }
        fake.onEvent?("arrive", fence.name, nil)
        fake.onEvent?("leave", fence.name, nil)
        try store.significant(true)
        fake.onEvent?("significant", nil, fake.fix)
        let restoredAdapter = FakeLocation()
        let restored = try LocationStore(adapter: restoredAdapter, url: url)
        try await restored.restore()
        guard restored.snapshot.records.map(\.kind) == ["current", "geofence", "arrive", "leave", "significant"], restoredAdapter.fences.count == 1, restoredAdapter.significantEnabled else { throw LocationFailure.invalid }
        try JSONEncoder().encode(restored.snapshot.records).write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
        try restored.remove(fence.name)
        try restored.significant(false)
        guard restored.snapshot.fences.isEmpty, restoredAdapter.fences.isEmpty, !restoredAdapter.significantEnabled else { throw LocationFailure.invalid }
        restoredAdapter.registrationFails = true
        do {
            try await restored.add(fence)
            throw LocationFailure.invalid
        } catch LocationFailure.unavailable { }
        guard restored.snapshot.fences.isEmpty else { throw LocationFailure.invalid }
        restoredAdapter.allowed = false
        do {
            try await restored.add(fence)
            throw LocationFailure.invalid
        } catch LocationFailure.permission { }
        guard restored.snapshot.fences.isEmpty else { throw LocationFailure.invalid }
        do {
            try await restored.current()
            throw LocationFailure.invalid
        } catch LocationFailure.permission { }
        try restored.acknowledge(Set(restored.snapshot.records.map(\.id)))
        let empty = try LocationStore(adapter: FakeLocation(), url: url)
        guard empty.snapshot.records.isEmpty else { throw LocationFailure.invalid }
        print("location_adapter_proof=pass")
    }
}
