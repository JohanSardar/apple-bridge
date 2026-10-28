import Foundation
import Vision
import ImageIO
import SourceSupport

public enum VisionQueries {
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        let data = try queryDocument(args)
        guard Set(args.object.keys).isSubset(of: ["data_base64", "file", "languages"]),
              let image = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 16000, height <= 16000, width * height <= 40000000 else { throw SourceFailure.invalid }
        try Task.checkCancellation()
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.automaticallyDetectsLanguage = true
        if !args["languages"].array.isEmpty { request.recognitionLanguages = args["languages"].array.map(\.string) }
        try VNImageRequestHandler(data: data).perform([request])
        try Task.checkCancellation()
        let lines: [JSON] = (request.results ?? []).compactMap { observation in
            guard let candidate = observation.topCandidates(1).first else { return nil }
            let box = observation.boundingBox
            return .object(["text": .string(candidate.string), "confidence": .number(Double(candidate.confidence)), "box": .object(["x": .number(box.minX), "y": .number(box.minY), "width": .number(box.width), "height": .number(box.height)])])
        }
        return .object(["text": .string(lines.map { $0["text"].string }.joined(separator: "\n")), "lines": .array(lines), "language": .string(detectLanguage(lines.map { $0["text"].string }.joined(separator: "\n")))])
    }
}

import NaturalLanguage

private func detectLanguage(_ text: String) -> String {
    NLLanguageRecognizer.dominantLanguage(for: text)?.rawValue ?? "und"
}
