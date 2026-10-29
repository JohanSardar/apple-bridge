import Foundation
import Vision
import SourceSupport

public actor VisionReadiness: Source {
    public nonisolated let name = "vision"
    private let generation = UUID().uuidString
    public init() {}
    public func open() async -> SourceReadiness {
        do {
            let languages = try VNRecognizeTextRequest().supportedRecognitionLanguages()
            guard !languages.isEmpty else { throw SourceFailure.unavailable }
            return SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete"), "language_count": .number(Double(languages.count))]))
        } catch {
            return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("unsupported")]), reason: "recognition_languages_unavailable")
        }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: [], scanID: "", scanStartedAt: "", scope: [], completionEvidence: "", blobHandles: [:], live: false)
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
