import Foundation
import UIKit

@MainActor
enum CaptureCapability {
    static let descriptor = Capability(id: "capture", title: "Camera and microphone", permission: "Face ID, camera and microphone", available: true, deferredHandlers: [
        "iphone.capture.photo": { job in try await deliver(job) },
        "iphone.capture.audio": { job in try await deliver(job) }
    ])

    static func deliver(_ job: PhoneJob) async throws -> Data? {
        guard ApprovalsCapability.enabled else { throw CaptureError.denied }
        try await ApprovalsCapability.deliver(job)
        return nil
    }

    static func decide(_ job: PhoneJob) async throws {
        guard UserDefaults.standard.bool(forKey: "capability.capture.enabled"), ApprovalsCapability.enabled else { throw CaptureError.denied }
        let kind = job.command.op == "iphone.capture.photo" ? "photo" : "audio"
        var result: PhoneResult
        do {
            let output = try await execute(job, kind: kind)
            result = PhoneResult(command_id: job.command.command_id, request_hash: job.request_hash, state: "observed", data: try JSONDecoder().decode(JSONValue.self, from: output))
        } catch {
            result = PhoneResult(command_id: job.command.command_id, request_hash: job.request_hash, state: "indeterminate", data: .null)
        }
        try NotificationsCapability.recordResult(result)
        UserDefaults.standard.removeObject(forKey: "approval." + job.command.command_id)
        await PushDelivery.shared.drain()
    }

    static func execute(_ job: PhoneJob, kind: String) async throws -> Data {
        guard UIApplication.shared.applicationState == .active else { throw CaptureError.denied }
        var seconds = 3
        if case .number(let value) = job.command.args["seconds"] { seconds = Int(value) }
        return try await CaptureFlow.run(kind: kind, seconds: seconds, adapter: DeviceCapture(expires: job.expires)) { bytes in
            guard job.expires > Int64(Date().timeIntervalSince1970), UserDefaults.standard.bool(forKey: "capability.capture.enabled") else { throw CaptureError.denied }
            let payload = try JSONSerialization.data(withJSONObject: ["command_id": job.command.command_id, "request_hash": job.request_hash, "content_type": bytes.contentType, "data": bytes.data.base64EncodedString()])
            return try await PairingClient.signedRequest(identity: DeviceIdentity.load(), path: "/v1/iphone/capture", payload: payload)
        }
    }
}
