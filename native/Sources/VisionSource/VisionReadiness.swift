import Foundation
import Vision
import SourceSupport

public actor VisionReadiness: Source {
    public nonisolated let name = "vision"
    public init() {}
    public func status() async -> JSON? {
        .object(["languages": .number(Double((try? VNRecognizeTextRequest().supportedRecognitionLanguages())?.count ?? 0))])
    }
}
