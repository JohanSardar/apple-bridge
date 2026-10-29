import Foundation

@MainActor
enum LocationCapability {
    static let adapter = CoreLocationAdapter()
    static var store: LocationStore?
    static var syncTask: Task<Void, Error>?

    static func load() throws -> LocationStore {
        if let store { return store }
        let directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let value = try LocationStore(adapter: adapter, url: directory.appendingPathComponent("location/records.json"))
        value.changed = { Task { try? await sync() } }
        store = value
        return value
    }

    static func resume() {
        guard UserDefaults.standard.bool(forKey: "capability.location.enabled") else { return }
        Task {
            do { try await load().restore() } catch { }
            try? await sync()
        }
    }

    static func sync() async throws {
        if let syncTask { try await syncTask.value; return }
        let task = Task { @MainActor in
            let store = try load()
            while !store.snapshot.records.isEmpty {
                let records = Array(store.snapshot.records.prefix(25))
                let identity = try DeviceIdentity.load()
                _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/location-records", payload: JSONEncoder().encode(records))
                try store.acknowledge(Set(records.map(\.id)))
            }
        }
        syncTask = task
        defer { syncTask = nil }
        try await task.value
    }

    static func handle(_ op: String, _ raw: Data) async throws -> Data {
        let store = try load()
        let args = try JSONSerialization.jsonObject(with: raw) as? [String: Any] ?? [:]
        switch op {
        case "iphone.location.current": try await store.current()
        case "iphone.geofence.add":
            guard let name = args["name"] as? String, let latitude = args["latitude"] as? Double, let longitude = args["longitude"] as? Double, let radius = args["radius"] as? Double else { throw LocationFailure.invalid }
            try await store.add(NamedGeofence(name: name, latitude: latitude, longitude: longitude, radius: radius))
        case "iphone.geofence.remove":
            guard let name = args["name"] as? String else { throw LocationFailure.invalid }
            try store.remove(name)
        case "iphone.geofence.list":
            for fence in store.snapshot.fences { try store.record("geofence", name: fence.name, fence: fence) }
        case "iphone.location.significant":
            guard let enabled = args["enabled"] as? Bool else { throw LocationFailure.invalid }
            try store.significant(enabled)
        default: throw LocationFailure.invalid
        }
        while !store.snapshot.records.isEmpty { try await sync() }
        return try JSONSerialization.data(withJSONObject: ["count": store.snapshot.fences.count, "significant": store.snapshot.significant])
    }

    static let descriptor = Capability(id: "location", title: "Location", permission: "Allow Always and Precise Location in Settings", available: true, handlers: Dictionary(uniqueKeysWithValues: ["iphone.location.current", "iphone.geofence.add", "iphone.geofence.remove", "iphone.geofence.list", "iphone.location.significant"].map { op in (op, { raw in try await handle(op, raw) } as IPhoneCommandHandler) }))
}
