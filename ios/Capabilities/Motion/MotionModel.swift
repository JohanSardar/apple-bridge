import Foundation

struct MotionSample: Codable, Sendable {
    let activity: String
    let confidence: String
    let start: Date
}

struct MotionDay: Codable, Sendable {
    let start: Date
    let end: Date
    let steps: Int
    let floorsUp: Int?
    let floorsDown: Int?
}

struct MotionSnapshot: Codable {
    let activities: [MotionSample]
    let days: [MotionDay]
    let omittedActivities: Int
}

@MainActor
protocol MotionAdapter {
    func currentActivity() async throws -> MotionSample
    func activities(from: Date, to: Date) async throws -> [MotionSample]
    func pedometer(from: Date, to: Date) async throws -> MotionDay
}

@MainActor
struct MotionService {
    let adapter: any MotionAdapter

    func snapshot(days: Int, now: Date = Date()) async throws -> MotionSnapshot {
        guard (1...7).contains(days) else { throw CocoaError(.coderInvalidValue) }
        let start = now.addingTimeInterval(-Double(days) * 86400)
        let activities = try await adapter.activities(from: start, to: now)
        var history: [MotionDay] = []
        for day in 0..<days {
            let from = start.addingTimeInterval(Double(day) * 86400)
            history.append(try await adapter.pedometer(from: from, to: min(from.addingTimeInterval(86400), now)))
        }
        var retained = Array(activities.suffix(128))
        while true {
            let result = MotionSnapshot(activities: retained, days: history, omittedActivities: activities.count - retained.count)
            if try JSONEncoder().encode(result).count <= 20000 { return result }
            guard !retained.isEmpty else { throw CocoaError(.coderInvalidValue) }
            retained.removeFirst()
        }
    }
}
