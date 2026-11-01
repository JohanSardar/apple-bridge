import Foundation

@MainActor
final class FakeHealth: HealthReading {
    let streams = ["steps", "walking_distance", "walking_speed", "walking_steadiness", "walking_asymmetry", "flights", "sleep", "heart_rate", "hrv", "resting_hr", "vo2_max", "workouts", "workout_route", "weight", "body_fat", "water", "nutrition", "mindful", "medication_dose", "medications"]
    var phase = 0
    var anchors: [String: Data?] = [:]
    func page(stream: String, anchor: Data?) async throws -> HealthPage {
        anchors[stream] = anchor
        if anchor == Data([UInt8(phase + 1)]) { return HealthPage(samples: [], deleted: [], anchor: anchor!, more: false) }
        if phase == 1 {
            return HealthPage(samples: [], deleted: [stream + "-1"], anchor: Data([2]), more: false)
        }
        return HealthPage(samples: [HealthSampleRecord(id: stream + "-1", type: stream, start: "2026-10-28T12:00:00Z", end: "2026-10-28T13:00:00Z", value: 10, unit: "count")], deleted: [], anchor: Data([1]), more: false)
    }
}

@main
struct HealthProof {
    @MainActor static func main() async throws {
        var bulk: [String] = []
        var individual: [String] = []
        try await HealthReadAuthorization.request(["steps", "medication_dose", "medications", "sleep"], indirect: { $0 == "medication_dose" }, perObject: { $0 == "medications" }, bulk: { bulk = $0 }, individual: { individual.append($0) })
        precondition(bulk == ["steps", "sleep"] && individual == ["medications"])
        do {
            try await HealthReadAuthorization.request(["steps", "medications"], perObject: { $0 == "medications" }, bulk: { _ in throw CocoaError(.userCancelled) }, individual: { _ in preconditionFailure() })
            preconditionFailure()
        } catch {}
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("health.json")
        let fake = FakeHealth()
        let capture = try HealthCapture(store: fake, url: url)
        var delivered: [HealthUpload] = []
        do {
            try await capture.capture { batch in
                delivered.append(batch)
                throw CocoaError(.fileWriteUnknown)
            }
            preconditionFailure()
        } catch {}
        precondition(delivered.count == 1 && delivered[0].sequence == 1)
        let restarted = try HealthCapture(store: fake, url: url)
        try await restarted.capture { delivered.append($0) }
        precondition(delivered.count == fake.streams.count + 1 && delivered[0].samples[0].id == delivered[1].samples[0].id)
        precondition(delivered.dropFirst().allSatisfy { $0.sequence == 1 })
        try await restarted.capture { _ in preconditionFailure() }
        fake.phase = 1
        try await restarted.capture { delivered.append($0) }
        precondition(delivered.count == fake.streams.count * 2 + 1 && delivered.suffix(fake.streams.count).allSatisfy { $0.sequence == 2 && $0.deleted.count == 1 })
        let final = try HealthCapture(store: fake, url: url)
        try await final.capture { _ in preconditionFailure() }
        precondition(fake.anchors.values.allSatisfy { $0 == Data([2]) })
        print("PASS")
    }
}
