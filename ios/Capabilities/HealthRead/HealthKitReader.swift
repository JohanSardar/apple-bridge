import CryptoKit
import Foundation
import HealthKit

@MainActor
final class HealthKitReader: HealthReading {
    let store = HKHealthStore()
    private var observers: [HKQuery] = []
    static let quantities: [(HKQuantityTypeIdentifier, String)] = [
        (.stepCount, "count"), (.distanceWalkingRunning, "m"), (.walkingSpeed, "m/s"),
        (.appleWalkingSteadiness, "%"), (.walkingAsymmetryPercentage, "%"), (.flightsClimbed, "count"),
        (.heartRate, "count/min"), (.heartRateVariabilitySDNN, "ms"), (.restingHeartRate, "count/min"),
        (.vo2Max, "ml/kg*min"), (.bodyMass, "kg"), (.bodyFatPercentage, "%"), (.dietaryWater, "ml"),
        (.dietaryEnergyConsumed, "kcal"), (.dietaryCarbohydrates, "g"), (.dietaryProtein, "g"),
        (.dietaryFatTotal, "g"), (.dietaryFatSaturated, "g"), (.dietaryFatMonounsaturated, "g"),
        (.dietaryFatPolyunsaturated, "g"), (.dietaryCholesterol, "mg"), (.dietarySodium, "mg"),
        (.dietarySugar, "g"), (.dietaryFiber, "g"), (.dietaryCaffeine, "mg"),
        (.dietaryCalcium, "mg"), (.dietaryIron, "mg"), (.dietaryPotassium, "mg"),
        (.dietaryVitaminA, "mcg"), (.dietaryVitaminB6, "mg"), (.dietaryVitaminB12, "mcg"),
        (.dietaryVitaminC, "mg"), (.dietaryVitaminD, "mcg"), (.dietaryVitaminE, "mg"),
        (.dietaryVitaminK, "mcg"), (.dietaryThiamin, "mg"), (.dietaryRiboflavin, "mg"),
        (.dietaryNiacin, "mg"), (.dietaryFolate, "mcg"), (.dietaryBiotin, "mcg"),
        (.dietaryPantothenicAcid, "mg"), (.dietaryPhosphorus, "mg"), (.dietaryIodine, "mcg"),
        (.dietaryMagnesium, "mg"), (.dietaryZinc, "mg"), (.dietarySelenium, "mcg"),
        (.dietaryCopper, "mg"), (.dietaryManganese, "mg"), (.dietaryChromium, "mcg"), (.dietaryMolybdenum, "mcg"), (.dietaryChloride, "mg")
    ]
    var types: [HKSampleType] {
        Self.quantities.compactMap { HKQuantityType.quantityType(forIdentifier: $0.0) } + [
            HKCategoryType(.sleepAnalysis), HKCategoryType(.mindfulSession), HKObjectType.workoutType(),
            HKSeriesType.workoutRoute(), HKObjectType.medicationDoseEventType()
        ]
    }
    var streams: [String] { types.map(\.identifier) + ["medications"] }

    func authorize() async throws {
        try await store.requestAuthorization(toShare: [], read: Set(types.map { $0 as HKObjectType } + [HKObjectType.userAnnotatedMedicationType()]))
    }

    func observe(wake: @escaping @MainActor () async -> Void) async throws {
        guard observers.isEmpty else { return }
        for type in types {
            let query = HKObserverQuery(sampleType: type, predicate: nil) { _, completion, error in
                let acknowledgement = HealthObserverAcknowledgement(completion)
                Task { @MainActor in
                    if error == nil { await wake() }
                    acknowledgement.finish()
                }
            }
            observers.append(query)
            store.execute(query)
            try? await store.enableBackgroundDelivery(for: type, frequency: .immediate)
        }
    }

    func stop() async {
        for query in observers { store.stop(query) }
        observers.removeAll()
        try? await store.disableAllBackgroundDelivery()
    }

    func page(stream: String, anchor: Data?) async throws -> HealthPage {
        if stream == "medications" { return try await medicationPage(anchor: anchor) }
        guard let type = types.first(where: { $0.identifier == stream }) else { throw HealthReadError.invalid }
        let previous = try anchor.map { try NSKeyedUnarchiver.unarchivedObject(ofClass: HKQueryAnchor.self, from: $0) } ?? nil
        let result: ([HKSample], [HKDeletedObject], HKQueryAnchor) = try await withCheckedThrowingContinuation { continuation in
            let query = HKAnchoredObjectQuery(type: type, predicate: nil, anchor: previous, limit: type is HKSeriesType ? 1 : 50) { _, samples, deleted, next, error in
                if let error { continuation.resume(throwing: error) }
                else if let next { continuation.resume(returning: (samples ?? [], deleted ?? [], next)) }
                else { continuation.resume(throwing: HealthReadError.invalid) }
            }
            store.execute(query)
        }
        var records: [HealthSampleRecord] = []
        for sample in result.0 { records.append(try await record(sample)) }
        return HealthPage(samples: records, deleted: result.1.map { $0.uuid.uuidString }, anchor: try NSKeyedArchiver.archivedData(withRootObject: result.2, requiringSecureCoding: true), more: result.0.count + result.1.count >= (type is HKSeriesType ? 1 : 50))
    }

    private func record(_ sample: HKSample) async throws -> HealthSampleRecord {
        let format = ISO8601DateFormatter()
        var r = HealthSampleRecord(id: sample.uuid.uuidString, type: sample.sampleType.identifier, start: format.string(from: sample.startDate), end: format.string(from: sample.endDate))
        r.details = ["source_bundle": .text(sample.sourceRevision.source.bundleIdentifier)]
        if let q = sample as? HKQuantitySample, let unit = Self.quantities.first(where: { $0.0.rawValue == r.type })?.1 {
            r.value = q.quantity.doubleValue(for: HKUnit(from: unit))
            r.unit = unit
        }
        if let category = sample as? HKCategorySample { r.category = category.value }
        if let workout = sample as? HKWorkout {
            r.details?["activity_type"] = .number(Double(workout.workoutActivityType.rawValue))
            r.details?["duration_seconds"] = .number(workout.duration)
            if let distance = workout.totalDistance { r.details?["distance_m"] = .number(distance.doubleValue(for: .meter())) }
            if let energy = workout.statistics(for: HKQuantityType(.activeEnergyBurned))?.sumQuantity() { r.details?["energy_kcal"] = .number(energy.doubleValue(for: .kilocalorie())) }
        }
        if let route = sample as? HKWorkoutRoute {
            let points: [HealthValue] = try await withCheckedThrowingContinuation { continuation in
                let buffer = RouteBuffer()
                let query = HKWorkoutRouteQuery(route: route) { _, locations, done, error in
                    if let error { continuation.resume(throwing: error); return }
                    buffer.points += (locations ?? []).map { .array([.number($0.coordinate.latitude), .number($0.coordinate.longitude), .number($0.altitude), .number($0.timestamp.timeIntervalSince1970)]) }
                    if done { continuation.resume(returning: buffer.points) }
                }
                store.execute(query)
            }
            r.details?["points_lat_lon_alt_time"] = .array(points)
            let workouts: [HKSample] = try await withCheckedThrowingContinuation { continuation in
                store.execute(HKSampleQuery(sampleType: .workoutType(), predicate: HKQuery.predicateForSamples(withStart: route.startDate.addingTimeInterval(-86400), end: route.endDate.addingTimeInterval(86400)), limit: HKObjectQueryNoLimit, sortDescriptors: nil) { _, samples, error in
                    if let error { continuation.resume(throwing: error) } else { continuation.resume(returning: samples ?? []) }
                })
            }
            for workout in workouts.compactMap({ $0 as? HKWorkout }) {
                let routes: [HKSample] = try await withCheckedThrowingContinuation { continuation in
                    store.execute(HKSampleQuery(sampleType: HKSeriesType.workoutRoute(), predicate: HKQuery.predicateForObjects(from: workout), limit: HKObjectQueryNoLimit, sortDescriptors: nil) { _, samples, error in
                        if let error { continuation.resume(throwing: error) } else { continuation.resume(returning: samples ?? []) }
                    })
                }
                if routes.contains(where: { $0.uuid == route.uuid }) { r.details?["workout_id"] = .text(workout.uuid.uuidString); break }
            }
        }
        if let dose = sample as? HKMedicationDoseEvent {
            r.category = dose.logStatus.rawValue
            r.details?["medication_id"] = .text(try Self.conceptID(dose.medicationConceptIdentifier))
            r.details?["schedule_type"] = .number(Double(dose.scheduleType.rawValue))
            r.unit = dose.unit.unitString
            r.value = dose.doseQuantity
        }
        return r
    }

    nonisolated private static func conceptID(_ identifier: HKHealthConceptIdentifier) throws -> String {
        let data = try NSKeyedArchiver.archivedData(withRootObject: identifier, requiringSecureCoding: true)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private func medicationPage(anchor: Data?) async throws -> HealthPage {
        let records: [HealthSampleRecord] = try await withCheckedThrowingContinuation { continuation in
            let buffer = MedicationBuffer()
            store.execute(HKUserAnnotatedMedicationQuery(predicate: nil, limit: HKObjectQueryNoLimit) { _, medication, done, error in
                if let error { continuation.resume(throwing: error); return }
                if let m = medication {
                    do {
                    buffer.records.append(HealthSampleRecord(id: try Self.conceptID(m.medication.identifier), type: "medications", start: "1970-01-01T00:00:00Z", end: "1970-01-01T00:00:00Z", details: ["name": .text(m.medication.displayText), "nickname": .text(m.nickname ?? ""), "archived": .bool(m.isArchived), "has_schedule": .bool(m.hasSchedule)]))
                    } catch { continuation.resume(throwing: error); return }
                }
                if done { continuation.resume(returning: buffer.records) }
            })
        }
        let sorted = records.sorted { $0.id < $1.id }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let old = try anchor.map { try JSONDecoder().decode([HealthSampleRecord].self, from: $0) } ?? []
        let ids = Set(sorted.map(\.id))
        let oldByID = Dictionary(uniqueKeysWithValues: old.map { ($0.id, $0) })
        var changed: [HealthSampleRecord] = []
        for record in sorted {
            if let prior = oldByID[record.id], try encoder.encode(prior) == encoder.encode(record) { continue }
            changed.append(record)
        }
        let removed = old.map(\.id).filter { !ids.contains($0) }
        let page = Array(changed.prefix(50))
        let deleted = Array(removed.prefix(50 - page.count))
        var committed = oldByID
        for record in page { committed[record.id] = record }
        for id in deleted { committed.removeValue(forKey: id) }
        let checkpoint = try encoder.encode(committed.values.sorted { $0.id < $1.id })
        return HealthPage(samples: page, deleted: deleted, anchor: checkpoint, more: changed.count + removed.count > 50)
    }
}

private final class HealthObserverAcknowledgement: @unchecked Sendable {
    private let completion: () -> Void
    init(_ completion: @escaping () -> Void) { self.completion = completion }
    func finish() { completion() }
}

private final class RouteBuffer: @unchecked Sendable { var points: [HealthValue] = [] }
private final class MedicationBuffer: @unchecked Sendable { var records: [HealthSampleRecord] = [] }
enum HealthReadError: Error { case invalid }
