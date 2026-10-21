import Foundation
import AVFoundation
import ImageIO
import UniformTypeIdentifiers

@MainActor
enum MediaTruth {
    static func observe(_ request: [String: Any]) async throws -> [String: Any] {
        guard let path = request["file"] as? String else { throw TruthFailure.invalid }
        let file = URL(fileURLWithPath: path).resolvingSymlinksInPath()
        guard file.path.hasPrefix("/private/tmp/ablab/") || file.path.hasPrefix("/tmp/ablab/") else { throw TruthFailure.invalid }
        if let source = CGImageSourceCreateWithURL(file as CFURL, nil), let identifier = CGImageSourceGetType(source), let type = UTType(identifier as String), type.conforms(to: .image), let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any], let width = properties[kCGImagePropertyPixelWidth as String] as? Int, let height = properties[kCGImagePropertyPixelHeight as String] as? Int {
            return ["status": "verified", "type": identifier as String, "width": width, "height": height]
        }
        let asset = AVURLAsset(url: file)
        let duration = try await asset.load(.duration).seconds
        guard duration.isFinite, duration > 0 else { throw TruthFailure.unavailable }
        var value: [String: Any] = ["status": "verified", "duration_ms": duration * 1000]
        for (name, kind) in [("video", AVMediaType.video), ("audio", AVMediaType.audio)] {
            guard let track = try await asset.loadTracks(withMediaType: kind).first else { continue }
            let formats = try await track.load(.formatDescriptions)
            value[name] = formats.map { format in
                let code = CMFormatDescriptionGetMediaSubType(format)
                return String(bytes: [UInt8(truncatingIfNeeded: code >> 24), UInt8(truncatingIfNeeded: code >> 16), UInt8(truncatingIfNeeded: code >> 8), UInt8(truncatingIfNeeded: code)], encoding: .ascii) ?? "unknown"
            }
            if name == "video" {
                let size = try await track.load(.naturalSize)
                let transform = try await track.load(.preferredTransform)
                value["width"] = Int(abs(size.applying(transform).width).rounded())
                value["height"] = Int(abs(size.applying(transform).height).rounded())
            }
        }
        return value
    }
}
