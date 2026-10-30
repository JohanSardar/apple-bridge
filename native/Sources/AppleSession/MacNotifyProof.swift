#if LAB
import Foundation
import UserNotifications
import MacSource
import SourceSupport

@MainActor
enum MacNotifyProof {
    static func run() async -> JSON {
        let center = UNUserNotificationCenter.current()
        let before = Set(await center.deliveredNotifications().map { $0.request.identifier })
        let tag = "[ab-test] " + UUID().uuidString
        let source = MacSource()
        var checks: [String: JSON] = [:]
        var identifier = ""
        do {
            let prepared = try await source.prepare(.object(["request_hash": .string(tag), "action": .object(["op": .string("mac.notify"), "args": .object(["title": .string(tag), "body": .string("Apple Bridge notification fixture")])])]))
            let result = try await source.dispatch(prepared)
            identifier = result["readback"]["identifier"].string
            checks["notification_observed"] = .bool(result["observed"].bool && result["readback"]["delivered"].bool)
            checks["delivered_content_matches"] = .bool(await center.deliveredNotifications().contains { $0.request.identifier == identifier && $0.request.content.title == tag })
        } catch { checks["notification_observed"] = .bool(false) }
        let owned = await center.deliveredNotifications().filter { $0.request.content.title == tag && !before.contains($0.request.identifier) }.map { $0.request.identifier }
        center.removeDeliveredNotifications(withIdentifiers: owned)
        center.removePendingNotificationRequests(withIdentifiers: owned + (identifier.isEmpty ? [] : [identifier]))
        for _ in 0..<50 {
            if await !center.deliveredNotifications().contains(where: { owned.contains($0.request.identifier) }) { break }
            try? await Task.sleep(for: .milliseconds(100))
        }
        let after = Set(await center.deliveredNotifications().map { $0.request.identifier })
        checks["owned_notification_removed"] = .bool(owned.allSatisfy { !after.contains($0) })
        checks["existing_notifications_preserved"] = .bool(before.isSubset(of: after))
        return .object(checks)
    }
}
#endif
