import Foundation

@MainActor
final class SetupPermissions {
    var requests: [String] = []
    var denied: String?
    var statuses: [String: String] = [:]
    func status() async -> [String: String] { statuses }
    func request(_ permission: SetupPermission) async throws {
        requests.append(permission.id)
        statuses[permission.id] = denied == permission.id ? "denied" : "granted"
    }
}
