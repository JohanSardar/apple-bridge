import Foundation

enum HealthLogError: Error { case invalid }

struct HealthLog: Codable, Equatable {
    let kind: String
    let value: Double
    let start: String
    let tag: String
    var activity: String?

    func validated() throws -> (Date, Date) {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let parsed = formatter.date(from: start) ?? ISO8601DateFormatter().date(from: start)
        guard ["water", "weight", "mindful", "workout"].contains(kind), value.isFinite, value > 0,
              !tag.isEmpty, tag.utf8.count <= 128,
              let date = parsed,
              kind != "workout" || ["walking", "running", "cycling", "other"].contains(activity ?? "") else { throw HealthLogError.invalid }
        let end = date.addingTimeInterval(kind == "mindful" || kind == "workout" ? value * 60 : 0)
        guard end.timeIntervalSince1970.isFinite else { throw HealthLogError.invalid }
        return (date, end)
    }
}

@MainActor
protocol HealthWriting {
    func save(_ log: HealthLog) async throws -> String
    func contains(id: String, log: HealthLog) async throws -> Bool
    func delete(id: String, log: HealthLog) async throws
}

@MainActor
enum HealthWriteProof {
    static func run(store: HealthWriting) async throws {
        for kind in ["water", "weight", "mindful", "workout"] {
            let log = HealthLog(kind: kind, value: 1, start: ISO8601DateFormatter().string(from: Date()), tag: "[ab-test]-" + UUID().uuidString, activity: kind == "workout" ? "walking" : nil)
            let id = try await store.save(log)
            do {
                guard try await store.contains(id: id, log: log) else { throw CocoaError(.fileReadUnknown) }
            } catch {
                try await store.delete(id: id, log: log)
                throw error
            }
            try await store.delete(id: id, log: log)
            guard try await !store.contains(id: id, log: log) else { throw CocoaError(.fileWriteUnknown) }
        }
    }
}
