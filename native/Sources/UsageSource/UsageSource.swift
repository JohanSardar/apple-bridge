import Foundation
import SourceSupport

@MainActor
public final class UsageSource: Source {
    public nonisolated let name = "usage"
    private let store = UsageStore(home: FileManager.default.homeDirectoryForCurrentUser)
    private var previous: [String: JSON] = [:]
    public init() {}
    public func open() async -> SourceReadiness {
        do {
            _ = try store.read()
            return SourceReadiness(state: "ready", coverage: .object(["fields": .string("partial"), "missing": .array([.string("settings_parity: unproven"), .string("lock evidence: loginwindow/ScreenSaver focus only; no display/lock Biome stream on 25G241"), .string("historical sleep gaps; open sessions; remote build metadata unavailable; remote batches may lag")])]))
        } catch {
            return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("partial"), "missing": .array([.string("biome_access_required")])]), reason: "biome_access_required")
        }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let biome = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Biome")
        let watcher = UsageWatcher(paths: [biome.appendingPathComponent("streams/restricted/App.InFocus").path, biome.appendingPathComponent("sync").path])
        defer { watcher.close() }
        for await _ in watcher.changes {
            try Task.checkCancellation()
            let started = timestamp()
            let observations = try store.read()
            let changes = observations.filter { previous[$0["collection"].string + "/" + $0["native_key"].string] != $0["data"] }
            try await sink.emit(source: name, generation: "biome-app-infocus-1", observations: changes, scanID: UUID().uuidString, scanStartedAt: started, scope: ["usage.device", "usage.app_day"], completionEvidence: "complete", blobHandles: [:], live: true)
            for observation in observations { previous[observation["collection"].string + "/" + observation["native_key"].string] = observation["data"] }
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
