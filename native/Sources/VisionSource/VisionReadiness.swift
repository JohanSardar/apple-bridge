import Foundation
import Vision
import SourceSupport

public actor VisionReadiness: Source {
    public nonisolated let name = "vision"
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
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
