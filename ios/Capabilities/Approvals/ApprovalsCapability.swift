import Foundation
import LocalAuthentication
import UserNotifications
import SwiftUI

@MainActor
enum ApprovalsCapability {
    static let descriptor = Capability(id: "approvals", title: "Face ID approvals", permission: "Notifications and Face ID", available: true)

    static var enabled: Bool { UserDefaults.standard.bool(forKey: "capability.approvals.enabled") }

    static func stored(_ id: String) -> PhoneJob? {
        ApprovalInbox.stored(id)
    }

    static func pending() -> [PhoneJob] {
        ApprovalInbox.pending()
    }

    static func deliver(_ job: PhoneJob) async throws {
        guard enabled else { return }
        if let previous = stored(job.command.command_id), previous.request_hash == job.request_hash, previous.command.op == job.command.op { return }
        let center = UNUserNotificationCenter.current()
        let actions = [UNNotificationAction(identifier: "approve", title: "Allow", options: [.foreground]), UNNotificationAction(identifier: "reject", title: "Deny", options: [.foreground, .destructive])]
        var categories = await center.notificationCategories()
        categories.insert(UNNotificationCategory(identifier: "bridge.approval", actions: actions, intentIdentifiers: []))
        center.setNotificationCategories(categories)
        let content = UNMutableNotificationContent()
        content.title = job.command.command_id.hasPrefix("connector:") ? "Connect to Apple Bridge" : "Apple Bridge approval"
        if case .string(let op) = job.command.args["body"] { content.body = op }
        else { content.body = job.command.op }
        content.categoryIdentifier = "bridge.approval"
        content.sound = .default
        content.userInfo = ["command_id": job.command.command_id, "request_hash": job.request_hash, "approval": true, "expires": job.expires]
        #if DEBUG
        if ShellDeliveryQualification.active { try ShellDeliveryQualification.shared.registerConnector(job) }
        #endif
        try await center.add(UNNotificationRequest(identifier: "approval." + job.command.command_id, content: content, trigger: nil))
        UserDefaults.standard.set(try JSONEncoder().encode(job), forKey: "approval." + job.command.command_id)
    }

    static func present(id: String, hash: String, action: String, expires: Int64) throws {
        guard enabled, expires > Int64(Date().timeIntervalSince1970), let job = stored(id), job.request_hash == hash else { return }
        ApprovalReview.shared.job = job
    }

    static func decide(id: String, hash: String, action: String, expires: Int64) async throws {
        guard enabled, let job = stored(id), job.request_hash == hash, ["approve", "reject"].contains(action), expires > Int64(Date().timeIntervalSince1970) else { return }
        if job.command.op.hasPrefix("iphone.capture."), action == "approve" {
            try await CaptureCapability.decide(job)
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ["approval." + id])
            return
        }
        let context = LAContext()
        do {
            guard try await context.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, localizedReason: "Confirm \(action) for this Apple Bridge command") else { throw LAError(.authenticationFailed) }
        } catch {
            #if DEBUG
            if ShellDeliveryQualification.active {
                let proof = ShellDeliveryQualification.shared
                if proof.request?.command == id { proof.receipt?.checks["face_id_succeeded"] = false }
                if proof.connectorID == id { proof.receipt?.checks["connector_face_id_succeeded"] = false }
                try? proof.save()
            }
            #endif
            throw error
        }
        #if DEBUG
        if ShellDeliveryQualification.active, ShellDeliveryQualification.shared.request?.command == id {
            ShellDeliveryQualification.shared.receipt?.checks["face_id_succeeded"] = true
            try ShellDeliveryQualification.shared.save()
        }
        #endif
        guard enabled, expires > Int64(Date().timeIntervalSince1970) else { return }
        if job.command.op.hasPrefix("iphone.capture.") {
            try NotificationsCapability.recordResult(PhoneResult(command_id: id, request_hash: hash, state: "rejected", data: .null))
            ApprovalInbox.remove(id)
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ["approval." + id])
            await PushDelivery.shared.drain()
            return
        }
        let payload = try JSONSerialization.data(withJSONObject: ["command_id": id, "request_hash": hash, "approve": action == "approve"])
        _ = try await PairingClient.signedRequest(identity: DeviceIdentity.load(), path: "/v1/iphone/approval", payload: payload)
        #if DEBUG
        if ShellDeliveryQualification.active, ShellDeliveryQualification.shared.connectorID == id {
            ShellDeliveryQualification.shared.receipt?.checks["connector_face_id_succeeded"] = true
            ShellDeliveryQualification.shared.receipt?.checks["connector_decision_uploaded"] = true
            try ShellDeliveryQualification.shared.save()
        }
        #endif
        ApprovalInbox.remove(id)
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ["approval." + id])
    }
}

@MainActor
@Observable
final class ApprovalReview {
    static let shared = ApprovalReview()
    var job: PhoneJob?
}

struct ApprovalReviewScreen: View {
    let job: PhoneJob
    @State private var busy = false
    @State private var status = ""
    var body: some View {
        NavigationStack {
            Form {
                if case .string(let summary) = job.command.args["body"] {
                    Text(summary).textSelection(.enabled).privacySensitive()
                }
                if job.command.op.hasPrefix("iphone.capture.") {
                    Text(job.command.op == "iphone.capture.photo" ? "Capture one photo and upload it to your gateway." : "Record audio and upload it to your gateway.")
                    if case .number(let seconds) = job.command.args["seconds"] { Text("Duration: \(Int(seconds)) seconds") }
                }
                Text(status)
                ForEach(["approve", "reject"], id: \.self) { action in
                    Button(job.command.command_id.hasPrefix("connector:") ? (action == "approve" ? "Allow" : "Deny") : action.capitalized, role: action == "reject" ? .destructive : nil) {
                        busy = true
                        Task {
                            do {
                                try await ApprovalsCapability.decide(id: job.command.command_id, hash: job.request_hash, action: action, expires: job.expires)
                                ApprovalReview.shared.job = nil
                            } catch { status = "Decision pending. Try again." }
                            busy = false
                        }
                    }.disabled(busy || !ApprovalsCapability.enabled)
                }
            }.navigationTitle(job.command.command_id.hasPrefix("connector:") ? "Review connection" : "Review command")
        }
    }
}
