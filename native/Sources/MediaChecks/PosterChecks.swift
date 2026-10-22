import Foundation
import ImageIO
import CoreGraphics
import CryptoKit
import UniformTypeIdentifiers
@_spi(Testing) import MediaDerivatives

struct PosterChecks {
    static func run(_ source: URL) async throws {
        let digest = SHA256.hash(data: try Data(contentsOf: source)).map { String(format: "%02x", $0) }.joined()
        let cache = MediaDerivatives.root.appendingPathComponent(digest)
        guard !FileManager.default.fileExists(atPath: cache.path) else { throw PosterFailure.checkpoint("fresh-cache") }
        defer { try? FileManager.default.removeItem(at: cache) }
        let initial = try await MediaDerivatives.make(source, digest: digest)
        guard let poster = initial.derivatives["poster"], let video = initial.derivatives["video"] else { throw PosterFailure.checkpoint("variants") }
        let image = try decode(MediaDerivatives.url(poster))
        guard image.width == 3456, image.height == 360 else { throw PosterFailure.checkpoint("panorama-resolution") }
        let contrast = try edgeContrast(image)
        guard contrast >= 0.5 else { throw PosterFailure.checkpoint("panorama-edge-contrast") }
        guard !MediaDerivatives.needsPosterUpgrade(initial) else { throw PosterFailure.checkpoint("fresh-policy") }
        let manifest = cache.appendingPathComponent("manifest.json")
        var stale = try JSONSerialization.jsonObject(with: Data(contentsOf: manifest)) as! [String: Any]
        var variants = stale["derivatives"] as! [String: Any]
        var old = variants["poster"] as! [String: Any]
        old["width"] = 288;old["height"] = 30;variants["poster"] = old;stale["derivatives"] = variants
        try JSONSerialization.data(withJSONObject: stale).write(to: manifest, options: .atomic)
        guard let cached = MediaDerivatives.cached(digest), MediaDerivatives.needsPosterUpgrade(cached) else { throw PosterFailure.checkpoint("stale-policy") }
        let offline = source.appendingPathExtension("offline")
        try FileManager.default.moveItem(at: source, to: offline)
        do {
            let fallback = try await MediaDerivatives.make(source, digest: digest)
            guard fallback.derivatives["poster"]?.width == 288, fallback.derivatives["video"]?.digest == video.digest else { throw PosterFailure.checkpoint("offline-fallback") }
        } catch {
            try? FileManager.default.moveItem(at: offline, to: source)
            throw error
        }
        try FileManager.default.moveItem(at: offline, to: source)
        let upgraded = try await MediaDerivatives.make(source, digest: digest)
        guard !MediaDerivatives.needsPosterUpgrade(upgraded), upgraded.derivatives["video"]?.digest == video.digest, upgraded.media.thumbhash == initial.media.thumbhash else { throw PosterFailure.checkpoint("cache-upgrade-preserves-media") }
        for (width, height) in [(1920, 200), (200, 1920), (320, 180), (180, 320), (1, 1), (100000, 1)] {
            let size = try MediaDerivatives.posterSize(width: width, height: height)
            guard size.width * size.height <= 1_500_000, min(size.width, size.height) >= (max(width, height) <= 10 * min(width, height) ? 360 : 1) else { throw PosterFailure.checkpoint("poster-budget") }
        }
        let live = try await liveStill()
        let metrics: [String: Any] = ["width": image.width, "height": image.height, "pixels": image.width * image.height, "edge_contrast": contrast, "edge_minimum": 0.5, "offline_fallback_preserved": true, "cache_upgrade_preserved_video_and_thumbhash": true, "live_key_still_mean_rgb_error": live]
        print(String(data: try JSONSerialization.data(withJSONObject: metrics, options: [.sortedKeys]), encoding: .utf8)!)
    }
    static func decode(_ url: URL) throws -> CGImage {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil), let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw PosterFailure.checkpoint("decode") }
        return image
    }
    static func pixels(_ image: CGImage, width: Int, height: Int) throws -> [UInt8] {
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        let success = bytes.withUnsafeMutableBytes { raw in
            guard let context = CGContext(data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
            context.interpolationQuality = .high
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard success else { throw PosterFailure.checkpoint("pixels") }
        return bytes
    }
    static func edgeContrast(_ image: CGImage) throws -> Double {
        let width = image.width, height = image.height, bytes = try pixels(image, width: image.width, height: image.height)
        func sample(_ x: Double, _ y: Int) -> Double {
            let position = x - 0.5, left = max(0, min(width - 1, Int(floor(position)))), right = min(width - 1, left + 1), fraction = position - floor(position)
            let a = (y * width + left) * 4, b = (y * width + right) * 4
            return (Double(bytes[a]) - Double(bytes[a + 2])) * (1 - fraction) + (Double(bytes[b]) - Double(bytes[b + 2])) * fraction
        }
        let scale = Double(height) / 360, edge = Double(width) / 2 - Double(height) * 0.4
        var contrast = 0.0
        for offset in -3...3 {
            var delta = 0.0
            for row in -2...2 {
                let y = max(0, min(height - 1, height / 2 + row)), center = edge + Double(offset) * scale
                delta += (sample(center + 1.5 * scale, y) - sample(center - 1.5 * scale, y)) / 510
            }
            contrast = max(contrast, delta / 5)
        }
        return contrast
    }
    static func liveStill() async throws -> Double {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-poster-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let identifier = UUID().uuidString, source = directory.appendingPathComponent("still.heic"), movie = directory.appendingPathComponent("motion.mov")
        let context = CGContext(data: nil, width: 1200, height: 600, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(CGColor(red: 1, green: 0.2, blue: 0, alpha: 1));context.fill(CGRect(x: 0, y: 0, width: 1200, height: 600))
        let destination = CGImageDestinationCreateWithURL(source as CFURL, UTType.heic.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyOrientation: 6, kCGImagePropertyMakerAppleDictionary: ["17": identifier]] as CFDictionary)
        guard CGImageDestinationFinalize(destination), MediaDerivatives.liveIdentifier(source) == identifier else { throw PosterFailure.checkpoint("live-fixture") }
        try await MediaChecks.movie(movie, small: true, identifier: identifier)
        let digest = SHA256.hash(data: try Data(contentsOf: source)).map { String(format: "%02x", $0) }.joined()
        let cache = MediaDerivatives.root.appendingPathComponent(digest)
        guard !FileManager.default.fileExists(atPath: cache.path) else { throw PosterFailure.checkpoint("fresh-live-cache") }
        defer { try? FileManager.default.removeItem(at: cache) }
        let result = try await MediaDerivatives.make(source, digest: digest, companions: [movie])
        guard result.media.live == true, result.derivatives["live"] != nil, let poster = result.derivatives["poster"] else { throw PosterFailure.checkpoint("live-pair") }
        let image = try decode(MediaDerivatives.url(poster))
        guard image.width == 360, image.height == 720 else { throw PosterFailure.checkpoint("live-resolution-and-orientation") }
        let bytes = try pixels(image, width: 1, height: 1)
        let mean = (abs(Double(bytes[0]) - 255) + abs(Double(bytes[1]) - 51) + Double(bytes[2])) / 3
        guard mean <= 10 else { throw PosterFailure.checkpoint("live-key-still-not-motion") }
        return mean
    }
}

private enum PosterFailure: Error { case checkpoint(String) }
