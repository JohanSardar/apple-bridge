import Foundation
import UserNotifications

@MainActor
enum NotificationsCapability {
    static let descriptor = Capability(id: "notifications", title: "Notifications", available: true, deferredHandlers: ["iphone.notify": deliver])

    struct Button: Codable {
        let id: String
        let title: String
    }

    struct Arguments: Decodable {
        let title: String
        let body: String?
        let buttons: [Button]?
        let thread: String?
        let time_sensitive: Bool?
    }

    static func requestPermission() async throws {
        _ = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
    }

    static func deliver(_ job: PhoneJob) async throws -> Data? {
        let args = try JSONDecoder().decode(Arguments.self, from: JSONEncoder().encode(job.command.args))
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional || settings.authorizationStatus == .ephemeral else { throw IdentityError.requestFailed }
        let content = UNMutableNotificationContent()
        content.title = args.title
        content.body = args.body ?? ""
        content.threadIdentifier = args.thread ?? ""
        content.sound = .default
        content.interruptionLevel = args.time_sensitive == true ? .timeSensitive : .active
        content.userInfo = ["command_id": job.command.command_id, "request_hash": job.request_hash, "buttons": (args.buttons ?? []).map(\.id)]
        let buttons = args.buttons ?? []
        if !buttons.isEmpty {
            let categoryID = "bridge.\(job.command.command_id)"
            var categories = await center.notificationCategories()
            let pending = await center.pendingNotificationRequests()
            let delivered = await center.deliveredNotifications()
            let active = Set(pending.map { $0.content.categoryIdentifier } + delivered.map { $0.request.content.categoryIdentifier })
            categories = Set(categories.filter { active.contains($0.identifier) })
            categories.insert(UNNotificationCategory(identifier: categoryID, actions: buttons.map { UNNotificationAction(identifier: $0.id, title: $0.title) }, intentIdentifiers: [], options: [.customDismissAction]))
            center.setNotificationCategories(categories)
            content.categoryIdentifier = categoryID
        }
        try await center.add(UNNotificationRequest(identifier: job.command.command_id, content: content, trigger: nil))
        return buttons.isEmpty ? Data("{\"delivered\":true}".utf8) : nil
    }

    private static var responseURL: URL {
        get throws {
            try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("notification-responses.json")
        }
    }

    static func responses() throws -> [PhoneResult] {
        guard FileManager.default.fileExists(atPath: try responseURL.path) else { return [] }
        return try JSONDecoder().decode([PhoneResult].self, from: Data(contentsOf: responseURL))
    }

    private static func save(_ values: [PhoneResult]) throws {
        try JSONEncoder().encode(values).write(to: responseURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    static func record(id: String, hash: String, buttons: [String], action: String) throws {
        guard !buttons.isEmpty else { return }
        guard buttons.contains(action) || action == UNNotificationDefaultActionIdentifier || action == UNNotificationDismissActionIdentifier else { return }
        let value: JSONValue = buttons.contains(action) ? .object(["action_id": .string(action)]) : .object(["dismissed": .bool(true)])
        var values = try responses()
        if !values.contains(where: { $0.command_id == id }) {
            values.append(PhoneResult(command_id: id, request_hash: hash, state: "observed", data: value))
            try save(values)
        }
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
    }

    static func removeResponse(_ id: String) throws {
        try save(responses().filter { $0.command_id != id })
    }
}
