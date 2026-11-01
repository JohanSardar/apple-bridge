#if DEBUG
import ActivityKit
import AppIntents
import CryptoKit
import Foundation
import Intents
import SwiftUI
import UserNotifications
import WidgetKit

struct RealRequest: Decodable {
    let run: String
    let scenario: String
    let code: String?
    let command: String?
}

struct ShellDeliveryBackup: Codable {
    let identity: DeviceIdentity
    let switches: [String: Bool]
    let absent: [String]
    let status: BridgeStatus
    let run: String?
    let command: String?
    var connector: String? = nil
}

struct RealReceipt: Codable {
    let run: String
    var done = false
    var checks: [String: Bool] = [:]
    var counts: [String: Int] = [:]
    var digests: [String: String] = [:]
}

@MainActor
@Observable
final class ShellDeliveryQualification {
    static let shared = ShellDeliveryQualification()
    static var active: Bool { ProcessInfo.processInfo.arguments.contains("--ab-real") || ProcessInfo.processInfo.arguments.contains("--ab-real-restore") }
    var receipt: RealReceipt?
    var request: RealRequest?
    var instruction = "Starting [ab-test] qualification"
    var shellCapability = false
    var shellDiagnostics = false
    var confirmed = false
    var intentCount = 0
    var connectorID: String?

    static var directory: URL { FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0] }

    static func statusDigest(_ status: BridgeStatus) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return SHA256.hash(data: try encoder.encode(status)).map { String(format: "%02x", $0) }.joined()
    }

    static func restoreBackup() async throws {
        let url = directory.appendingPathComponent("ab-real-backup.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        let backup = try JSONDecoder().decode(ShellDeliveryBackup.self, from: Data(contentsOf: url))
        if let run = backup.run {
            for activity in Activity<AgentTaskAttributes>.activities where activity.attributes.taskID == run {
                await activity.end(nil, dismissalPolicy: .immediate)
            }
            guard !Activity<AgentTaskAttributes>.activities.contains(where: { $0.attributes.taskID == run && $0.activityState == .active }) else { throw IdentityError.requestFailed }
        }
        for id in [backup.command, backup.connector].compactMap({ $0 }) {
            let center = UNUserNotificationCenter.current()
            let ids = [id, "approval." + id]
            center.removeDeliveredNotifications(withIdentifiers: ids)
            center.removePendingNotificationRequests(withIdentifiers: ids)
            let categories = await center.notificationCategories()
            center.setNotificationCategories(Set(categories.filter { $0.identifier != "bridge." + id }))
            UserDefaults.standard.removeObject(forKey: "approval." + id)
            if ApprovalReview.shared.job?.command.command_id == id { ApprovalReview.shared.job = nil }
            try NotificationsCapability.removeResponse(id)
            let cacheURL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("command-receipts.json")
            if FileManager.default.fileExists(atPath: cacheURL.path) {
                var cache = try JSONDecoder().decode([String: Execution].self, from: Data(contentsOf: cacheURL))
                cache.removeValue(forKey: id)
                try JSONEncoder().encode(cache).write(to: cacheURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            }
            let delivered = await center.deliveredNotifications()
            let pending = await center.pendingNotificationRequests()
            guard !delivered.contains(where: { ids.contains($0.request.identifier) }), !pending.contains(where: { ids.contains($0.identifier) }) else { throw IdentityError.requestFailed }
        }
        try backup.identity.save()
        for (key, value) in backup.switches { UserDefaults.standard.set(value, forKey: key) }
        for key in backup.absent { UserDefaults.standard.removeObject(forKey: key) }
        backup.status.save()
        WidgetCenter.shared.reloadTimelines(ofKind: "BridgeStatus")
        let digest = try statusDigest(backup.status)
        guard try statusDigest(BridgeStatus.load()) == digest else { throw IdentityError.requestFailed }
        if let run = backup.run {
            let restored = RealReceipt(run: run, done: true, checks: ["state_restored": true, "resources_cleaned": true], digests: ["widget": digest])
            try JSONEncoder().encode(restored).write(to: directory.appendingPathComponent("ab-real-restored.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        }
        try FileManager.default.removeItem(at: url)
    }

    func save() throws {
        guard let receipt else { return }
        try JSONEncoder().encode(receipt).write(to: Self.directory.appendingPathComponent("ab-real-result.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    func wait(_ condition: @MainActor () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(180)
        while !condition() && Date() < deadline { try await Task.sleep(for: .milliseconds(250)) }
    }

    func registerConnector(_ job: PhoneJob) throws {
        guard request?.scenario == "APP-APPROVE-REAL", receipt?.counts["phase"] == 3, job.command.command_id.hasPrefix("connector:") else { return }
        let url = Self.directory.appendingPathComponent("ab-real-backup.json")
        var backup = try JSONDecoder().decode(ShellDeliveryBackup.self, from: Data(contentsOf: url))
        backup.connector = job.command.command_id
        try JSONEncoder().encode(backup).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        connectorID = job.command.command_id
        if case .string(let body) = job.command.args["body"] {
            receipt?.checks["connector_prompt_code_and_scopes"] = job.request_hash.count == 4 && job.request_hash.allSatisfy(\.isNumber) && body.contains("Code " + job.request_hash) && body.contains("messages:read") && body.contains("[ab-test] iPhone connection")
        }
        try save()
    }

    func recordIntent(_ value: String) {
        guard Self.active, request?.scenario == "APP-INTENTS-REAL" else { return }
        intentCount += 1
        receipt?.checks["shortcut_pong"] = value.contains("\"pong\":true")
        try? save()
    }

    func run() async {
        guard request == nil else { return }
        let idleTimer = UIApplication.shared.isIdleTimerDisabled
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = idleTimer }
        var preferences: [String: Any] = [:]
        var activity: Activity<AgentTaskAttributes>?
        let keys = ["notifications", "approvals", "widgets"].map { "capability.\($0).enabled" }
        do {
            try await Self.restoreBackup()
            for key in keys { preferences[key] = UserDefaults.standard.object(forKey: key) ?? NSNull() }
            let arguments = ProcessInfo.processInfo.arguments
            if let index = arguments.firstIndex(of: "--ab-real-restore"), arguments.count > index + 1 {
                try? FileManager.default.removeItem(at: Self.directory.appendingPathComponent("ab-real-request.json"))
                let restoredURL = Self.directory.appendingPathComponent("ab-real-restored.json")
                let restored = try JSONDecoder().decode(RealReceipt.self, from: Data(contentsOf: restoredURL))
                guard restored.run == arguments[index + 1], restored.checks["state_restored"] == true, restored.checks["resources_cleaned"] == true, restored.digests["widget"] == (try Self.statusDigest(BridgeStatus.load())) else { throw IdentityError.requestFailed }
                try JSONEncoder().encode(restored).write(to: Self.directory.appendingPathComponent("ab-real-acknowledged.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                instruction = "[ab-test] State and tagged resources restored."
                return
            }
            let input = Self.directory.appendingPathComponent("ab-real-request.json")
            let request = try JSONDecoder().decode(RealRequest.self, from: Data(contentsOf: input))
            self.request = request
            let identity = try DeviceIdentity.load()
            let backup = ShellDeliveryBackup(identity: identity, switches: preferences.compactMapValues { $0 as? Bool }, absent: preferences.filter { $0.value is NSNull }.map(\.key), status: BridgeStatus.load(), run: request.run, command: request.command)
            var backupURL = Self.directory.appendingPathComponent("ab-real-backup.json")
            try JSONEncoder().encode(backup).write(to: backupURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try backupURL.setResourceValues(values)
            try FileManager.default.removeItem(at: input)
            receipt = RealReceipt(run: request.run)
            try save()
            if let code = request.code {
                let identity = try await PairingClient.pair(code)
                let status = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/status")
                receipt?.checks["signed_pair_status"] = try JSONDecoder().decode(PairedDevice.self, from: status).id == identity.deviceID
                if let token = identity.pushToken { await PushDelivery.shared.register(token: token) }
                try save()
            }
            receipt?.counts["phase"] = 1
            try save()
            switch request.scenario {
            case "APP-SHELL-REAL":
                instruction = "Open Capability and Diagnostics below, inspect the fields, then confirm."
                try await wait { self.shellCapability && self.shellDiagnostics && self.confirmed }
                receipt?.checks["capability_fields_observed"] = shellCapability && confirmed
                receipt?.checks["diagnostics_observed"] = shellDiagnostics && confirmed
                receipt?.counts["capabilities"] = Capabilities.all.count
            case "APP-PAIR-REAL":
                break
            case "FOC-PHONE-SYNC":
                instruction = "Tap Allow to share Focus status if requested. Keep Share Across Devices enabled in Focus settings."
                let center = INFocusStatusCenter.default
                if center.authorizationStatus == .notDetermined {
                    _ = await withCheckedContinuation { continuation in center.requestAuthorization { continuation.resume(returning: $0) } }
                }
                receipt?.checks["focus_authorized"] = center.authorizationStatus == .authorized
                receipt?.checks["phone_initially_inactive"] = center.focusStatus.isFocused == false
                guard center.authorizationStatus == .authorized, center.focusStatus.isFocused == false else { throw IdentityError.requestFailed }
                receipt?.checks["ready"] = true
                try save()
                instruction = "Waiting for the Pro to enable and restore Do Not Disturb."
                let enabled = await QualificationWait.until { center.focusStatus.isFocused == true }
                receipt?.checks["phone_focus_enabled"] = enabled
                try save()
                if enabled { receipt?.checks["phone_focus_restored"] = await QualificationWait.until { center.focusStatus.isFocused == false } }
                else { receipt?.checks["phone_focus_restored"] = false }
            case "APP-CMD-REAL", "APP-NOTIFY-REAL", "APP-APPROVE-REAL":
                if request.scenario != "APP-CMD-REAL" {
                    for key in keys.prefix(2) { UserDefaults.standard.set(true, forKey: key) }
                    instruction = "Allow notification alerts if requested."
                    let before = await UNUserNotificationCenter.current().notificationSettings()
                    receipt?.counts["notification_prompt_requested"] = before.authorizationStatus == .notDetermined ? 1 : 0
                    try save()
                    try await NotificationsCapability.requestPermission()
                    let settings = await UNUserNotificationCenter.current().notificationSettings()
                    receipt?.checks["notification_authorized"] = settings.authorizationStatus == .authorized
                }
                instruction = request.scenario == "APP-APPROVE-REAL" ? "Open the [ab-test] approval notification, tap Approve, and complete Face ID." : "Tap [ab-test] Confirm on the notification if shown."
                receipt?.counts["phase"] = 2
                receipt?.checks["ready"] = true
                try save()
                let before = PushDelivery.shared.wakeCount
                let deadline = Date().addingTimeInterval(180)
                var uploaded = false
                while Date() < deadline {
                    if request.scenario == "APP-APPROVE-REAL", receipt?.checks["face_id_succeeded"] == false { break }
                    if let command = request.command,
                       let data = try? Data(contentsOf: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("command-receipts.json")),
                       let cache = try? JSONDecoder().decode([String: Execution].self, from: data),
                       cache[command]?.uploaded == true {
                        uploaded = true
                        break
                    }
                    try await Task.sleep(for: .milliseconds(250))
                }
                receipt?.checks["signed_result_uploaded"] = uploaded
                if request.scenario == "APP-APPROVE-REAL" {
                    receipt?.counts["phase"] = 3
                    instruction = "Open [ab-test] iPhone connection, match the code in the Pro instructions, tap Allow, and complete Face ID."
                    try save()
                    try await wait { self.receipt?.checks["connector_decision_uploaded"] == true || self.receipt?.checks["connector_face_id_succeeded"] == false }
                }
                receipt?.counts["apns_wakes"] = PushDelivery.shared.wakeCount - before
                if request.scenario == "APP-CMD-REAL" { receipt?.checks["apns_wake"] = PushDelivery.shared.wakeCount > before }
            case "APP-LIVE-REAL":
                receipt?.checks["activities_authorized"] = ActivityAuthorizationInfo().areActivitiesEnabled
                let attributes = AgentTaskAttributes(taskID: request.run, title: "[ab-test] Live qualification")
                activity = try Activity.request(attributes: attributes, content: ActivityContent(state: AgentTaskAttributes.ContentState(status: "[ab-test] Started", progress: 0), staleDate: nil), pushType: nil)
                receipt?.checks["activity_started"] = activity?.activityState == .active
                await activity?.update(ActivityContent(state: AgentTaskAttributes.ContentState(status: "[ab-test] Updated", progress: 0.5), staleDate: nil))
                receipt?.checks["activity_updated"] = activity?.content.state.progress == 0.5
                WidgetsCapability.status("[ab-test] Widget qualification")
                receipt?.checks["widget_shared_status"] = BridgeStatus.load().status == "[ab-test] Widget qualification"
                instruction = "Inspect [ab-test] Updated on the Lock Screen and [ab-test] Widget qualification in the Bridge status widget, return here and confirm."
                try await wait { self.confirmed }
                receipt?.checks["live_and_widget_observed"] = confirmed
                await activity?.end(nil, dismissalPolicy: .immediate)
                receipt?.checks["activity_ended"] = activity?.activityState == .ended || activity?.activityState == .dismissed
            case "APP-INTENTS-REAL":
                instruction = "In Shortcuts save [ab-test] Ping iPhone using Apple Bridge’s Check Bridge Status action, run it, then return here. Delete the test shortcut afterward."
                try await wait { self.intentCount > 0 }
                receipt?.checks["saved_shortcut_invoked"] = intentCount > 0
                receipt?.counts["intent_invocations"] = intentCount
                confirmed = false
                receipt?.counts["phase"] = 2
                try save()
                instruction = "Delete [ab-test] Ping iPhone from Shortcuts, return here, and confirm cleanup."
                try await wait { self.confirmed }
                receipt?.checks["shortcut_cleaned_up"] = confirmed
            default:
                receipt?.checks["supported_scenario"] = false
            }
        } catch {
            receipt?.checks["native_execution"] = false
            receipt?.digests["error"] = SHA256.hash(data: Data(String(describing: error).utf8)).map { String(format: "%02x", $0) }.joined()
        }
        if let activity, activity.activityState == .active { await activity.end(nil, dismissalPolicy: .immediate) }
        do {
            try await Self.restoreBackup()
            receipt?.checks["state_restored"] = true
            receipt?.checks["resources_cleaned"] = true
            if request?.code != nil { receipt?.checks["identity_restored"] = true }
        } catch {
            receipt?.checks["state_restored"] = false
            receipt?.checks["resources_cleaned"] = false
            if request?.code != nil { receipt?.checks["identity_restored"] = false }
        }
        receipt?.done = true
        try? save()
        instruction = "[ab-test] Qualification finished."
    }
}

struct ShellDeliveryQualificationScreen: View {
    @Environment(\.scenePhase) private var scenePhase
    @Bindable var proof = ShellDeliveryQualification.shared
    @State private var pairing = PairingState()
    @Bindable private var approval = ApprovalReview.shared
    var body: some View {
        NavigationStack {
            List {
                Text(proof.instruction)
                if proof.request?.scenario == "APP-SHELL-REAL", let capability = Capabilities.all.first {
                    NavigationLink("Capability") {
                        CapabilityScreen(capability: capability).onAppear { proof.shellCapability = true }
                    }
                    NavigationLink("Diagnostics") {
                        DiagnosticsScreen(pairing: pairing).onAppear { proof.shellDiagnostics = true }
                    }
                }
                Button("Confirm observations") { proof.confirmed = true }
            }.navigationTitle("[ab-test] Real qualification")
        }
        .sheet(isPresented: Binding(get: { approval.job != nil }, set: { if !$0 { approval.job = nil } })) {
            if let job = approval.job { ApprovalReviewScreen(job: job) }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active, proof.receipt?.done == false, proof.receipt?.checks["ready"] == true { Task { await PushDelivery.shared.drain() } }
        }
        .task { await proof.run() }
    }
}
#endif
