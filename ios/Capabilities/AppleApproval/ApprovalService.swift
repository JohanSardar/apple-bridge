import Foundation

enum ApprovalFeature: String, CaseIterable {
    case wallet
    case criticalAlerts
    case locationPush
    case fallDetection
}

enum AppleApprovalError: Error {
    case disabled
    case needsAppleApproval
    case needsAuthorization
    case needsWatchCompanion
    case invalidArguments
}

@MainActor
protocol AppleApprovalAdapter {
    func perform(_ command: String, raw: Data) async throws -> Data
}

@MainActor
enum AppleApprovalService {
    static func execute(_ command: String, raw: Data, enabled: Bool, approved: Bool, adapter: any AppleApprovalAdapter) async throws -> Data {
        guard approved else { throw AppleApprovalError.needsAppleApproval }
        guard enabled else { throw AppleApprovalError.disabled }
        return try await adapter.perform(command, raw: raw)
    }
}
