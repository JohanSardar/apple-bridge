import Foundation
import CoreGraphics
import ImageIO
import AVFoundation
import Darwin

extension MediaDerivatives {
    @_spi(Testing) public static func posterSize(width: Int, height: Int) throws -> CGSize {
        guard width > 0, height > 0 else { throw MediaFailure.decode }
        let ratio = Double(max(width, height)) / Double(min(width, height))
        let short = max(1, min(360, floor(sqrt(1_500_000 / ratio))))
        let long = max(1, floor(min(1_500_000, short * ratio)))
        return width >= height ? CGSize(width: long, height: short) : CGSize(width: short, height: long)
    }
    static func posterImage(_ image: CGImage, size: CGSize) throws -> CGImage {
        let width = Int(size.width), height = Int(size.height)
        if image.width == width && image.height == height { return image }
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw MediaFailure.encode }
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(origin: .zero, size: size))
        guard let result = context.makeImage() else { throw MediaFailure.encode }
        return result
    }
    public static func needsPosterUpgrade(_ result: MediaResult) -> Bool {
        guard result.media.live == true || result.derivatives["video"] != nil,
              let width = result.media.width, let height = result.media.height,
              let size = try? posterSize(width: width, height: height) else { return false }
        guard let poster = result.derivatives["poster"], let w = poster.width, let h = poster.height, w > 0, h > 0 else { return true }
        return Double(w) < size.width || Double(h) < size.height || w > 1_500_000 / h
    }
    static func writePoster(_ image: CGImage, size: CGSize, directory: URL, alpha: Bool = false) async throws -> MediaVariant {
        try await MediaWork.yield()
        let poster = try posterImage(image, size: size)
        let file = directory.appendingPathComponent(alpha ? "poster.png" : "poster.jpg")
        try await MediaWork.yield()
        try write(poster, to: file, alpha: alpha, quality: 0.8)
        return try await variant(file, mime: alpha ? "image/png" : "image/jpeg", width: poster.width, height: poster.height)
    }
    static func stillPoster(_ source: CGImageSource, width: Int, height: Int, directory: URL) async throws -> MediaVariant {
        try await MediaWork.stage("live_poster")
        let size = try posterSize(width: width, height: height)
        let image = try thumbnail(source, maximum: Int(max(size.width, size.height)))
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any] ?? [:]
        return try await writePoster(image, size: size, directory: directory, alpha: properties[kCGImagePropertyHasAlpha as String] as? Bool == true)
    }
    static func moviePoster(_ asset: AVAsset, width: Int, height: Int, directory: URL) async throws -> MediaVariant {
        try await MediaWork.stage("video_poster")
        let size = try posterSize(width: width, height: height)
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = size
        return try await writePoster(try await generator.image(at: .zero).image, size: size, directory: directory)
    }
    static func posterSourceAvailable(_ input: URL) -> Bool {
        var metadata = stat()
        return Darwin.lstat(input.path, &metadata) == 0 && metadata.st_flags & UInt32(SF_DATALESS) == 0
    }
    static func upgradedPoster(_ input: URL, existing: MediaResult, directory: URL) async throws -> MediaResult {
        guard posterSourceAvailable(input), let width = existing.media.width, let height = existing.media.height else { return existing }
        var result = existing
        if existing.media.live == true {
            guard let source = CGImageSourceCreateWithURL(input as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { throw MediaFailure.decode }
            result.derivatives["poster"] = try await stillPoster(source, width: width, height: height, directory: directory)
        } else {
            result.derivatives["poster"] = try await moviePoster(AVURLAsset(url: input), width: width, height: height, directory: directory)
        }
        return result
    }
}
