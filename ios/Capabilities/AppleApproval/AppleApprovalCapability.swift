import Foundation
import FinanceKit
import UserNotifications
import CoreLocation

@MainActor
enum AppleApprovalCapability {
    static let descriptors: [Capability] = ApprovalFeature.allCases.map { feature in
        let title: String
        let commands: [String]
        switch feature {
        case .wallet: title = "Wallet / FinanceKit"; commands = ["iphone.wallet.account", "iphone.wallet.transaction"]
        case .criticalAlerts: title = "Critical alerts"; commands = ["iphone.critical.alert"]
        case .locationPush: title = "Location push"; commands = ["iphone.location.push"]
        case .fallDetection: title = "Fall detection"; commands = ["iphone.fall.detection"]
        }
        let approved = approved(feature)
        let supported = feature != .fallDetection
        return Capability(id: feature.rawValue, title: title, permission: approved ? (supported ? "Needs system authorization" : "limited: needs Watch companion") : (supported ? "limited: needs Apple approval" : "limited: needs Apple approval; needs Watch companion"), available: approved && supported, handlers: Dictionary(uniqueKeysWithValues: commands.map { command in
            (command, { raw in
                try await AppleApprovalService.execute(command, raw: raw, enabled: UserDefaults.standard.bool(forKey: "capability.\(feature.rawValue).enabled"), approved: approved, adapter: PhoneAppleApprovalAdapter())
            } as IPhoneCommandHandler)
        }))
    }

    static func approved(_ feature: ApprovalFeature) -> Bool {
        Bundle.main.object(forInfoDictionaryKey: "ABApproved_\(feature.rawValue)") as? Bool == true
    }
}

@MainActor
final class PhoneAppleApprovalAdapter: AppleApprovalAdapter {
    private let locationManager = CLLocationManager()

    func perform(_ command: String, raw: Data) async throws -> Data {
        switch command {
        case "iphone.wallet.account", "iphone.wallet.transaction":
            guard FinanceStore.isDataAvailable(.financialData), try await FinanceStore.shared.authorizationStatus() == .authorized else { throw AppleApprovalError.needsAuthorization }
            struct Request: Decodable { let device_id: String; let limit: Int?; let offset: Int? }
            let request = try JSONDecoder().decode(Request.self, from: raw)
            let limit = request.limit ?? 10
            let offset = request.offset ?? 0
            guard (1...50).contains(limit), (0...100000).contains(offset) else { throw AppleApprovalError.invalidArguments }
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            let result: Data
            if command == "iphone.wallet.account" {
                result = try encoder.encode(await FinanceStore.shared.accounts(query: AccountQuery(limit: limit, offset: offset)))
            } else {
                result = try encoder.encode(await FinanceStore.shared.transactions(query: TransactionQuery(limit: limit, offset: offset)))
            }
            guard result.count <= 18000 else { throw AppleApprovalError.invalidArguments }
            return result
        case "iphone.critical.alert":
            struct Request: Decodable { let device_id: String; let title: String; let body: String? }
            let request = try JSONDecoder().decode(Request.self, from: raw)
            guard !request.title.isEmpty, request.title.count <= 128, (request.body ?? "").count <= 1024 else { throw AppleApprovalError.invalidArguments }
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            guard settings.authorizationStatus == .authorized, settings.criticalAlertSetting == .enabled else { throw AppleApprovalError.needsAuthorization }
            let content = UNMutableNotificationContent()
            content.title = request.title
            content.body = request.body ?? ""
            content.interruptionLevel = .critical
            content.sound = .defaultCritical
            try await center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
            return Data("{\"delivered\":true}".utf8)
        case "iphone.location.push":
            guard LocationPushState.group != nil, LocationPushState.enabled, locationManager.authorizationStatus == .authorizedAlways else { throw AppleApprovalError.needsAuthorization }
            return try await withCheckedThrowingContinuation { continuation in
                locationManager.startMonitoringLocationPushes { token, error in
                    if let error { continuation.resume(throwing: error) }
                    else if let token {
                        do {
                            struct Registration: Encodable { let token: String; let snapshot: LocationPushSnapshot? }
                            let snapshot = LocationPushState.snapshotURL.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(LocationPushSnapshot.self, from: $0) }
                            continuation.resume(returning: try JSONEncoder().encode(Registration(token: token.map { String(format: "%02x", $0) }.joined(), snapshot: snapshot)))
                        }
                        catch { continuation.resume(throwing: error) }
                    } else { continuation.resume(throwing: AppleApprovalError.needsAuthorization) }
                }
            }
        case "iphone.fall.detection":
            throw AppleApprovalError.needsWatchCompanion
        default:
            throw AppleApprovalError.invalidArguments
        }
    }
}
