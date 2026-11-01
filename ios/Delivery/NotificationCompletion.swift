import Foundation

enum NotificationCompletion {
    nonisolated static func deliver(_ operation: @escaping @MainActor @Sendable () async -> Void, completion: @escaping @Sendable () -> Void) {
        Task { @MainActor in
            await operation()
            completion()
        }
    }
}
