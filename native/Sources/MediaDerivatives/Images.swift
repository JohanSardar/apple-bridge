import Foundation
import ImageIO
import UniformTypeIdentifiers
import CoreGraphics

extension MediaDerivatives {
    static func thumbnail(_ source: CGImageSource, maximum: Int) throws -> CGImage {
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: maximum, kCGImageSourceShouldCacheImmediately: true] as CFDictionary) else { throw MediaFailure.decode }
        return image
    }
    static func images(_ source: CGImageSource, directory: URL, sticker: Bool) throws -> MediaResult {
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any] ?? [:]
        let width = properties[kCGImagePropertyPixelWidth as String] as? Int ?? 0
        let height = properties[kCGImagePropertyPixelHeight as String] as? Int ?? 0
        let orientation = properties[kCGImagePropertyOrientation as String] as? Int ?? 1
        guard width > 0, height > 0 else { throw MediaFailure.decode }
        var result = MediaResult()
        result.media.width = orientation >= 5 ? height : width
        result.media.height = orientation >= 5 ? width : height
        let small = try thumbnail(source, maximum: 100)
        result.media.thumbhash = try placeholder(small)
        let gif = CGImageSourceGetType(source).map { ($0 as String) == UTType.gif.identifier } ?? false
        for (name, maximum, quality) in sticker ? [("display", 1600, 0.85)] : gif ? [("thumb", 400, 0.8)] : [("thumb", 400, 0.8), ("display", 1600, 0.85)] {
            let image = try thumbnail(source, maximum: maximum)
            let alpha = sticker || properties[kCGImagePropertyHasAlpha as String] as? Bool == true
            let file = directory.appendingPathComponent(name + (alpha ? ".png" : ".jpg"))
            try write(image, to: file, alpha: alpha, quality: quality)
            result.derivatives[name] = try variant(file, mime: alpha ? "image/png" : "image/jpeg", width: image.width, height: image.height)
        }
        return result
    }
    static func write(_ image: CGImage, to file: URL, alpha: Bool, quality: Double) throws {
        guard let destination = CGImageDestinationCreateWithURL(file as CFURL, (alpha ? UTType.png : UTType.jpeg).identifier as CFString, 1, nil) else { throw MediaFailure.encode }
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: quality, kCGImagePropertyOrientation: 1] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw MediaFailure.encode }
    }
    static func placeholder(_ image: CGImage) throws -> String {
        var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
        guard let context = CGContext(data: &bytes, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { throw MediaFailure.decode }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        for i in stride(from: 0, to: bytes.count, by: 4) where bytes[i + 3] > 0 {
            let alpha = Int(bytes[i + 3])
            for c in 0..<3 { bytes[i + c] = UInt8(min(255, Int(bytes[i + c]) * 255 / alpha)) }
        }
        guard let hash = ThumbHash.encode(width: image.width, height: image.height, rgba: bytes) else { throw MediaFailure.encode }
        return hash.base64EncodedString()
    }
}
