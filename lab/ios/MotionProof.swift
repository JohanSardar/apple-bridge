import Foundation

@MainActor
final class FakeMotion: MotionAdapter {
    var ranges: [(Date, Date)] = []
    var denied = false
    var transitions = 4
    func currentActivity() async throws -> MotionSample {
        if denied { throw CocoaError(.fileReadNoPermission) }
        return MotionSample(activity: "walking", confidence: "high", start: Date())
    }
    func activities(from: Date, to: Date) async throws -> [MotionSample] {
        if denied { throw CocoaError(.fileReadNoPermission) }
        return (0..<transitions).map { MotionSample(activity: ["walking", "running", "driving", "still"][$0 % 4], confidence: "high", start: from.addingTimeInterval(Double($0))) }
    }
    func pedometer(from: Date, to: Date) async throws -> MotionDay {
        ranges.append((from, to))
        return MotionDay(start: from, end: to, steps: 100, floorsUp: 2, floorsDown: 1)
    }
}

@main
struct MotionProof {
    @MainActor static func main() async throws {
        let fake = FakeMotion()
        let live = try await fake.currentActivity()
        precondition(live.activity == "walking")
        let now = Date(timeIntervalSince1970: 1800000000)
        let result = try await MotionService(adapter: fake).snapshot(days: 7, now: now)
        precondition(result.days.count == 7 && result.activities.count == 4)
        precondition(result.days.reduce(0) { $0 + $1.steps } == 700)
        precondition(result.days.allSatisfy { $0.floorsUp == 2 && $0.floorsDown == 1 })
        precondition(fake.ranges.first!.0 == now.addingTimeInterval(-7 * 86400) && fake.ranges.last!.1 == now)
        for index in 1..<7 { precondition(fake.ranges[index - 1].1 == fake.ranges[index].0) }
        do { _ = try await MotionService(adapter: fake).snapshot(days: 8); preconditionFailure() } catch {}
        fake.denied = true
        do { _ = try await MotionService(adapter: fake).snapshot(days: 1); preconditionFailure() } catch {}
        let encoded = try JSONEncoder().encode(result)
        let decoded = try JSONDecoder().decode(MotionSnapshot.self, from: encoded)
        precondition(decoded.days.count == 7)
        fake.denied = false
        fake.transitions = 5000
        let large = try await MotionService(adapter: fake).snapshot(days: 7, now: now)
        let payload = try JSONEncoder().encode(large)
        precondition(payload.count <= 20000 && large.omittedActivities == 4872)
        precondition(large.activities.count == 128 && large.days.count == 7)
        let next = try await fake.currentActivity()
        precondition(next.activity == "walking")
        if CommandLine.arguments.count > 1 { try payload.write(to: URL(fileURLWithPath: CommandLine.arguments[1])) }
        print("PASS 8")
    }
}
