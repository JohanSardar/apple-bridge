import Foundation

@MainActor
enum QualificationWait {
    static func until(timeout: TimeInterval = 180, interval: Duration = .milliseconds(250), _ ready: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !ready() {
            guard Date() < deadline, !Task.isCancelled else { return false }
            do { try await Task.sleep(for: interval) }
            catch { return false }
        }
        return !Task.isCancelled
    }
}
