import ActivityKit
import Foundation
import WidgetKit

@MainActor
enum WidgetsCapability {
    static let descriptor = Capability(id: "widgets", title: "Live Activities and widgets", permission: "Enable to share ActivityKit push tokens", available: true)
    private static var observers: [Task<Void, Never>] = []
    private static var activityIDs: Set<String> = []

    static func resume() {
        guard observers.isEmpty, UserDefaults.standard.bool(forKey: "capability.widgets.enabled") else { return }
        observers.append(Task { @MainActor in
            for await token in Activity<AgentTaskAttributes>.pushToStartTokenUpdates {
                await register(token, task: "_start")
            }
        })
        observers.append(Task { @MainActor in
            for await activity in Activity<AgentTaskAttributes>.activityUpdates {
                observe(activity)
            }
        })
        for activity in Activity<AgentTaskAttributes>.activities { observe(activity) }
    }

    private static func observe(_ activity: Activity<AgentTaskAttributes>) {
        guard activityIDs.insert(activity.id).inserted else { return }
        observers.append(Task { @MainActor in await registerActivity(activity) })
        observers.append(Task { @MainActor in
            for await state in activity.activityStateUpdates {
                if state == .ended || state == .dismissed {
                    await register(Data(), task: activity.attributes.taskID)
                    activityIDs.remove(activity.id)
                    return
                }
            }
        })
    }

    static func disable() {
        for observer in observers { observer.cancel() }
        observers = []
        activityIDs = []
        Task {
            if let identity = try? DeviceIdentity.load() {
                _ = try? await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/live-disable")
            }
            for activity in Activity<AgentTaskAttributes>.activities {
                await activity.end(nil, dismissalPolicy: .immediate)
            }
        }
    }

    private static func registerActivity(_ activity: Activity<AgentTaskAttributes>) async {
        for await token in activity.pushTokenUpdates {
            await register(token, task: activity.attributes.taskID)
        }
    }

    private static func register(_ token: Data, task: String) async {
        while !Task.isCancelled && UserDefaults.standard.bool(forKey: "capability.widgets.enabled") {
            do {
                let identity = try DeviceIdentity.load()
                let payload = try JSONSerialization.data(withJSONObject: ["task_id": task, "token": token.map { String(format: "%02x", $0) }.joined()])
                _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/live-token", payload: payload)
                return
            } catch {
                try? await Task.sleep(for: .seconds(30))
            }
        }
    }

    static func status(_ value: String) {
        BridgeStatus(status: value, updated: Date()).save()
        WidgetCenter.shared.reloadTimelines(ofKind: "BridgeStatus")
    }
}
