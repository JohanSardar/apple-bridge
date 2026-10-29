import Foundation

@MainActor
final class FakeHealthWriter: HealthWriting {
    var samples: [String: HealthLog] = [:]
    var saved = 0
    var deleted = 0
    func save(_ log: HealthLog) async throws -> String {
        _ = try log.validated()
        let id = UUID().uuidString
        samples[id] = log
        saved += 1
        return id
    }
    func contains(id: String, log: HealthLog) async throws -> Bool { samples[id] == log }
    func delete(id: String, log: HealthLog) async throws {
        precondition(log.tag.hasPrefix("[ab-test]"))
        samples.removeValue(forKey: id)
        deleted += 1
    }
}

@main
struct HealthWriteScenario {
    @MainActor static func main() async throws {
        for timestamp in ["2026-10-29T12:00:00Z", "2026-10-29T12:00:00.123Z"] {
            _ = try HealthLog(kind: "water", value: 1, start: timestamp, tag: "[ab-test]").validated()
        }
        let store = FakeHealthWriter()
        try await HealthWriteProof.run(store: store)
        precondition(store.saved == 4 && store.deleted == 4 && store.samples.isEmpty)
        print("PASS")
    }
}
