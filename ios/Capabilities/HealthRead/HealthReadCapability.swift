import CryptoKit
import Foundation
import HealthKit
import Observation

@MainActor
@Observable
final class HealthReadController {
    static let shared = HealthReadController()
    var status = "Not requested"
    private let reader = HealthKitReader()
    private var capture: HealthCapture?
    private var captureDevice: String?

    func enable() async {
        guard HKHealthStore.isHealthDataAvailable() else { status = "Health unavailable"; return }
        do {
            try await reader.authorize()
            UserDefaults.standard.set(true, forKey: "capability.healthread.enabled")
            await resume()
        } catch { status = "Authorization incomplete" }
    }

    func resume() async {
        guard UserDefaults.standard.bool(forKey: "capability.healthread.enabled") else { return }
        do {
            let identity = try DeviceIdentity.load()
            guard let device = identity.deviceID else { status = "Pairing required"; return }
            if capture == nil || captureDevice != device {
                let directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                let key = SHA256.hash(data: Data(device.utf8)).map { String(format: "%02x", $0) }.joined()
                capture = try HealthCapture(store: reader, url: directory.appendingPathComponent("health-\(key).json"))
                captureDevice = device
            }
            try await reader.observe { await self.refresh() }
            await refresh()
        } catch { status = "Background delivery pending" }
    }

    func disable() async {
        UserDefaults.standard.set(false, forKey: "capability.healthread.enabled")
        await reader.stop()
        status = "Disabled"
    }

    func refresh() async {
        guard UserDefaults.standard.bool(forKey: "capability.healthread.enabled") else { return }
        do {
            guard let capture else { return }
            let identity = try DeviceIdentity.load()
            try await capture.capture { batch in
                guard UserDefaults.standard.bool(forKey: "capability.healthread.enabled") else { throw HealthReadError.invalid }
                _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/health", payload: JSONEncoder().encode(batch))
            }
            status = "Capture complete; Health controls read access"
        } catch { status = "Capture pending; check pairing and Health access" }
    }
}

enum HealthReadCapability {
    static let descriptor = Capability(id: "healthread", title: "Health read", permission: "Managed in Health", available: HKHealthStore.isHealthDataAvailable())
}
