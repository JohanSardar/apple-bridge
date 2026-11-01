import Foundation
import HealthKit
import Observation

@MainActor
final class HealthKitWriter: HealthWriting {
    let store = HKHealthStore()
    static let shared = HealthKitWriter()
    private var proofSamples: [String: HKSample] = [:]
    private let retainForProof: Bool

    init(retainForProof: Bool = false) { self.retainForProof = retainForProof }

    func type(_ kind: String) throws -> HKSampleType {
        switch kind {
        case "water": return HKQuantityType(.dietaryWater)
        case "weight": return HKQuantityType(.bodyMass)
        case "mindful": return HKCategoryType(.mindfulSession)
        case "workout": return HKObjectType.workoutType()
        default: throw HealthLogError.invalid
        }
    }

    func authorize() async throws {
        let types = try Set(["water", "weight", "mindful", "workout"].map { try type($0) })
        try await store.requestAuthorization(toShare: types, read: Set(types.map { $0 as HKObjectType }))
    }

    func save(_ log: HealthLog) async throws -> String {
        let (start, end) = try log.validated()
        let sampleType = try type(log.kind)
        guard store.authorizationStatus(for: sampleType) == .sharingAuthorized else { throw CocoaError(.fileWriteNoPermission) }
        let metadata: [String: Any] = [HKMetadataKeyExternalUUID: log.tag, HKMetadataKeyWasUserEntered: true]
        let sample: HKSample
        switch log.kind {
        case "water", "weight":
            sample = HKQuantitySample(type: sampleType as! HKQuantityType, quantity: HKQuantity(unit: HKUnit(from: log.kind == "water" ? "ml" : "kg"), doubleValue: log.value), start: start, end: end, metadata: metadata)
        case "mindful":
            sample = HKCategorySample(type: HKCategoryType(.mindfulSession), value: HKCategoryValue.notApplicable.rawValue, start: start, end: end, metadata: metadata)
        default:
            let activities: [String: HKWorkoutActivityType] = ["walking": .walking, "running": .running, "cycling": .cycling, "other": .other]
            sample = HKWorkout(activityType: activities[log.activity!]!, start: start, end: end, duration: end.timeIntervalSince(start), totalEnergyBurned: nil, totalDistance: nil, metadata: metadata)
        }
        try await store.save(sample)
        if retainForProof && log.tag.hasPrefix("[ab-test]") { proofSamples[sample.uuid.uuidString] = sample }
        return sample.uuid.uuidString
    }

    private func samples(id: String, log: HealthLog) async throws -> [HKSample] {
        guard let uuid = UUID(uuidString: id) else { throw HealthLogError.invalid }
        let sampleType = try type(log.kind)
        let predicate = NSCompoundPredicate(andPredicateWithSubpredicates: [HKQuery.predicateForObject(with: uuid), HKQuery.predicateForObjects(withMetadataKey: HKMetadataKeyExternalUUID, allowedValues: [log.tag])])
        return try await withCheckedThrowingContinuation { continuation in
            store.execute(HKSampleQuery(sampleType: sampleType, predicate: predicate, limit: 1, sortDescriptors: nil) { _, samples, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: samples ?? []) }
            })
        }
    }

    func cleanupTaggedSamples() async throws -> Int {
        guard retainForProof else { throw CocoaError(.fileWriteNoPermission) }
        var deleted = 0
        for kind in ["water", "weight", "mindful", "workout"] {
            let sampleType = try type(kind)
            func tagged() async throws -> [HKSample] {
                let values: [HKSample] = try await withCheckedThrowingContinuation { continuation in
                    store.execute(HKSampleQuery(sampleType: sampleType, predicate: HKQuery.predicateForObjects(from: HKSource.default()), limit: HKObjectQueryNoLimit, sortDescriptors: nil) { _, samples, error in
                        if let error { continuation.resume(throwing: error) }
                        else { continuation.resume(returning: samples ?? []) }
                    })
                }
                return values.filter { ($0.metadata?[HKMetadataKeyExternalUUID] as? String)?.hasPrefix("[ab-test]") == true && $0.sourceRevision.source.bundleIdentifier == Bundle.main.bundleIdentifier }
            }
            let matches = try await tagged()
            if !matches.isEmpty { try await store.delete(matches); deleted += matches.count }
            guard try await tagged().isEmpty else { throw CocoaError(.fileWriteUnknown) }
        }
        return deleted
    }

    func contains(id: String, log: HealthLog) async throws -> Bool {
        try await !samples(id: id, log: log).isEmpty
    }

    func delete(id: String, log: HealthLog) async throws {
        guard log.tag.hasPrefix("[ab-test]") else { throw CocoaError(.fileWriteNoPermission) }
        let matches = proofSamples[id].map { [$0] } ?? []
        guard matches.allSatisfy({ $0.metadata?[HKMetadataKeyExternalUUID] as? String == log.tag }) else { throw CocoaError(.fileWriteNoPermission) }
        if !matches.isEmpty { try await store.delete(matches) }
        proofSamples.removeValue(forKey: id)
    }
}

@MainActor
enum HealthWriteCapability {
    static let descriptor = Capability(id: "healthwrite", title: "Health write", permission: "Health write access required", available: HKHealthStore.isHealthDataAvailable(), handlers: [
        "iphone.health.log": { raw in
            let log = try JSONDecoder().decode(HealthLog.self, from: raw)
            let id = try await HealthKitWriter.shared.save(log)
            return try JSONEncoder().encode(["id": id])
        }
    ])
}

@MainActor
@Observable
final class HealthWriteController {
    static let shared = HealthWriteController()
    var status = "Not requested"
    func authorize() async {
        do {
            try await HealthKitWriter.shared.authorize()
            status = "Authorization complete; Health controls write access"
        } catch { status = "Authorization incomplete" }
    }
    func prove() async {
        do {
            try await HealthWriteProof.run(store: HealthKitWriter(retainForProof: true))
            status = "PASS: 4 tagged samples written, read and deleted"
        } catch { status = "Proof incomplete; check Health access and cleanup" }
    }
}
