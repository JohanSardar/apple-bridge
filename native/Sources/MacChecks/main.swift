import Foundation
import MacSource
import SourceSupport

@main
struct MacChecks {
    @MainActor
    static func main() async throws {
        let source = MacSource()
        let state = try source.state()
        for key in ["mac.power/system", "mac.display/system", "mac.network/system"] {
            guard case .object = state[key] ?? .null else { throw SourceFailure.corrupt }
        }
        let collections = Set(state.keys.map { String($0.split(separator: "/", maxSplits: 1)[0]) })
        guard collections.isSubset(of: ["mac.audio_device", "mac.display", "mac.power", "mac.network", "mac.app"]) else { throw SourceFailure.corrupt }
        guard state.values.allSatisfy({ if case .object = $0 { return true } else { return false } }) else { throw SourceFailure.corrupt }
        final class Calls: @unchecked Sendable {
            var values: [Bool] = []
        }
        let calls = Calls()
        let watcher = Task { @MainActor in
            try? await source.observe { live in calls.values.append(live) }
        }
        let deadline = Date().addingTimeInterval(10)
        while calls.values.isEmpty, Date() < deadline { try await Task.sleep(for: .milliseconds(50)) }
        watcher.cancel()
        _ = await watcher.value
        guard calls.values.first == false else { throw SourceFailure.corrupt }
        print("mac state passed=true keys=\(state.count)")
    }
}
