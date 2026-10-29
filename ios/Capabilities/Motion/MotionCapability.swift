import CoreMotion
import Foundation

@MainActor
final class CoreMotionAdapter: MotionAdapter {
    private let motion = CMMotionActivityManager()
    private let pedometer = CMPedometer()

    func currentActivity() async throws -> MotionSample {
        guard CMMotionActivityManager.isActivityAvailable(), CMMotionActivityManager.authorizationStatus() != .denied, CMMotionActivityManager.authorizationStatus() != .restricted else { throw CocoaError(.featureUnsupported) }
        return try await withCheckedThrowingContinuation { continuation in
            var finished = false
            motion.startActivityUpdates(to: .main) { [self] value in
                guard !finished, let value else { return }
                finished = true
                motion.stopActivityUpdates()
                continuation.resume(returning: MotionSample(activity: value.automotive ? "driving" : value.running ? "running" : value.walking ? "walking" : value.cycling ? "cycling" : value.stationary ? "still" : "unknown", confidence: value.confidence == .high ? "high" : value.confidence == .medium ? "medium" : "low", start: value.startDate))
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [self] in
                guard !finished else { return }
                finished = true
                motion.stopActivityUpdates()
                continuation.resume(throwing: CocoaError(.featureUnsupported))
            }
        }
    }

    func activities(from: Date, to: Date) async throws -> [MotionSample] {
        guard CMMotionActivityManager.isActivityAvailable(), CMMotionActivityManager.authorizationStatus() != .denied, CMMotionActivityManager.authorizationStatus() != .restricted else { throw CocoaError(.featureUnsupported) }
        return try await withCheckedThrowingContinuation { continuation in
            motion.queryActivityStarting(from: from, to: to, to: .main) { values, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: (values ?? []).map { value in
                    MotionSample(activity: value.automotive ? "driving" : value.running ? "running" : value.walking ? "walking" : value.cycling ? "cycling" : value.stationary ? "still" : "unknown", confidence: value.confidence == .high ? "high" : value.confidence == .medium ? "medium" : "low", start: value.startDate)
                })
            }
        }
    }

    func pedometer(from: Date, to: Date) async throws -> MotionDay {
        guard CMPedometer.isStepCountingAvailable(), CMPedometer.authorizationStatus() != .denied, CMPedometer.authorizationStatus() != .restricted else { throw CocoaError(.featureUnsupported) }
        return try await withCheckedThrowingContinuation { continuation in
            pedometer.queryPedometerData(from: from, to: to) { value, error in
                if let error { continuation.resume(throwing: error); return }
                guard let value else { continuation.resume(throwing: CocoaError(.coderReadCorrupt)); return }
                continuation.resume(returning: MotionDay(start: value.startDate, end: value.endDate, steps: value.numberOfSteps.intValue, floorsUp: value.floorsAscended?.intValue, floorsDown: value.floorsDescended?.intValue))
            }
        }
    }
}

@MainActor
enum MotionCapability {
    static let descriptor = Capability(id: "motion", title: "Motion", permission: "Motion & Fitness permission required", available: CMMotionActivityManager.isActivityAvailable(), handlers: [
        "iphone.motion.activity": { _ in
            return try JSONEncoder().encode(await CoreMotionAdapter().currentActivity())
        },
        "iphone.motion.history": { raw in
            struct Arguments: Decodable { let days: Int }
            let args = try JSONDecoder().decode(Arguments.self, from: raw)
            return try JSONEncoder().encode(await MotionService(adapter: CoreMotionAdapter()).snapshot(days: args.days))
        },
    ])
}
