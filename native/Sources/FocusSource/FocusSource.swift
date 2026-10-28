import Foundation
import Intents
import SourceSupport

@MainActor
public final class FocusSource: Source {
    public nonisolated let name = "focus"
    private let generation = UUID().uuidString
    public init() {}
    private var coverage: JSON { .object(["fields": .string("partial"), "missing": .array([.string("current Focus name and Set Focus pack unqualified")])]) }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "limited", coverage: coverage, reason: "Focus authorization is read only; pack actions unverified")
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        var previous: JSON?
        while !Task.isCancelled {
            let center = INFocusStatusCenter.default
            let authorization: String
            switch center.authorizationStatus {
            case .authorized: authorization = "authorized"
            case .denied: authorization = "denied"
            case .restricted: authorization = "restricted"
            default: authorization = "not_determined"
            }
            let focused: JSON = center.authorizationStatus == .authorized ? center.focusStatus.isFocused.map { .bool($0) } ?? .null : .null
            let data: JSON = .object(["focused": focused, "name": .null, "authorization": .string(authorization)])
            if data != previous {
                let started = timestamp()
                let observation: JSON = .object(["kind": .string("upsert"), "collection": .string("focus.status"), "native_key": .string("current"), "scoped_alias": .string("focus/status/current"), "observed_at": .string(started), "data": data])
                try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: [observation], scanID: UUID().uuidString, scanStartedAt: started, scope: ["focus.status"], completionEvidence: "complete", blobHandles: [:], live: false)
                previous = data
            }
            try await Task.sleep(for: .seconds(30))
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.capabilityUnproven }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.capabilityUnproven }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
