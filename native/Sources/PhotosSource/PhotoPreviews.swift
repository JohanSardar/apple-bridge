import Foundation
import AppKit
import CoreGraphics
import CoreText
import ImageIO
import UniformTypeIdentifiers
@preconcurrency import Photos
import SourceSupport

let photoImageByteLimit = 600 << 10

func photoPreview(_ args: JSON) async throws -> JSON {
    try fields(args, ["asset_id", "max_px", "network_access", "include_hidden"])
    let size = try integer(args, "max_px", default: 512, maximum: 1536, minimum: 64)
    let network = try boolean(args, "network_access", default: false)
    let photo = try visibleAsset(string(args, "asset_id"), includeHidden: boolean(args, "include_hidden", default: false))
    let image = try await renderedImage(photo, longestSide: size, network: network)
    let canvas = try drawingContext(width: image.width, height: image.height)
    canvas.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard let flattened = canvas.makeImage() else { throw SourceFailure.unavailable }
    var result = try encodedImage(flattened).object
    result["asset_id"] = .string(photo.localIdentifier)
    result["type"] = .string(mediaTypeName(photo))
    result["created_at"] = photo.creationDate.map { .string(timestamp($0)) } ?? .null
    result["original_width"] = .number(Double(photo.pixelWidth))
    result["original_height"] = .number(Double(photo.pixelHeight))
    return .object(result)
}

func photoContactSheet(_ args: JSON) async throws -> JSON {
    try fields(args, ["asset_ids", "columns", "cell_px", "network_access", "include_hidden"])
    guard case .array(let ids) = args["asset_ids"], !ids.isEmpty, ids.count <= 30 else { throw SourceFailure.invalid }
    let network = try boolean(args, "network_access", default: false)
    let hidden = try boolean(args, "include_hidden", default: false)
    let photos = try ids.map { value -> PHAsset in
        guard case .string(let id) = value else { throw SourceFailure.invalid }
        return try visibleAsset(id, includeHidden: hidden)
    }
    guard Set(photos.map(\.localIdentifier)).count == photos.count else { throw SourceFailure.invalid }
    let columns = try integer(args, "columns", default: min(photos.count, 5), maximum: 8, minimum: 1)
    let cell = try integer(args, "cell_px", default: 256, maximum: 384, minimum: 96)
    let rows = (photos.count + columns - 1) / columns
    let gap = 4
    let width = columns * cell + (columns + 1) * gap
    let height = rows * cell + (rows + 1) * gap
    let canvas = try drawingContext(width: width, height: height)
    canvas.setFillColor(CGColor(gray: 0.12, alpha: 1))
    canvas.fill(CGRect(x: 0, y: 0, width: width, height: height))
    var cells: [JSON] = []
    for (index, photo) in photos.enumerated() {
        try Task.checkCancellation()
        let column = index % columns
        let row = index / columns
        let frame = CGRect(x: gap + column * (cell + gap), y: height - (row + 1) * (cell + gap), width: cell, height: cell)
        var available = true
        do {
            let image = try await renderedImage(photo, longestSide: cell, network: network)
            let scale = min(Double(cell) / Double(image.width), Double(cell) / Double(image.height), 1)
            let drawn = CGSize(width: Double(image.width) * scale, height: Double(image.height) * scale)
            canvas.draw(image, in: CGRect(x: frame.midX - drawn.width / 2, y: frame.midY - drawn.height / 2, width: drawn.width, height: drawn.height))
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            available = false
            canvas.setFillColor(CGColor(gray: 0.25, alpha: 1))
            canvas.fill(frame)
        }
        drawLabel(String(index + 1), in: canvas, at: frame, cell: cell)
        cells.append(.object(["number": .number(Double(index + 1)), "asset_id": .string(photo.localIdentifier), "type": .string(mediaTypeName(photo)), "created_at": photo.creationDate.map { .string(timestamp($0)) } ?? .null, "rendered": .bool(available)]))
    }
    guard let sheet = canvas.makeImage() else { throw SourceFailure.unavailable }
    var result = try encodedImage(sheet).object
    result["cells"] = .array(cells)
    result["columns"] = .number(Double(columns))
    return .object(result)
}

private func visibleAsset(_ id: String, includeHidden: Bool) throws -> PHAsset {
    let photo = try asset(id)
    guard includeHidden || !photo.isHidden else { throw SourceFailure.invalid }
    guard photo.mediaType == .image || photo.mediaType == .video else { throw SourceFailure.unsupported }
    return photo
}

private func mediaTypeName(_ photo: PHAsset) -> String {
    [PHAssetMediaType.image: "image", .video: "video", .audio: "audio"][photo.mediaType] ?? "unknown"
}

private final class ImageRequest: @unchecked Sendable {
    private let lock = NSLock()
    private var id: PHImageRequestID?
    private var cancelled = false

    func start(_ body: () -> PHImageRequestID) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard !cancelled else { return false }
        id = body()
        return true
    }

    func cancel() {
        lock.lock(); defer { lock.unlock() }
        cancelled = true
        if let id { PHImageManager.default().cancelImageRequest(id) }
    }
}

private func renderedImage(_ photo: PHAsset, longestSide: Int, network: Bool) async throws -> CGImage {
    let options = PHImageRequestOptions()
    options.deliveryMode = .highQualityFormat
    options.resizeMode = .exact
    options.isNetworkAccessAllowed = network
    options.isSynchronous = false
    options.version = .current
    let target = CGSize(width: longestSide, height: longestSide)
    let request = ImageRequest()
    let image: CGImage = try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<CGImage, Error>) in
            let resumed = ResumeOnce()
            let started = request.start {
                PHImageManager.default().requestImage(for: photo, targetSize: target, contentMode: .aspectFit, options: options) { result, info in
                    if (info?[PHImageResultIsDegradedKey] as? Bool) == true { return }
                    guard resumed.claim() else { return }
                    if (info?[PHImageCancelledKey] as? Bool) == true {
                        continuation.resume(throwing: CancellationError())
                    } else if let result, let cgImage = result.cgImage(forProposedRect: nil, context: nil, hints: nil) {
                        continuation.resume(returning: cgImage)
                    } else {
                        continuation.resume(throwing: SourceFailure.unavailable)
                    }
                }
            }
            if !started, resumed.claim() { continuation.resume(throwing: CancellationError()) }
        }
    } onCancel: {
        request.cancel()
    }
    try Task.checkCancellation()
    let scale = min(Double(longestSide) / Double(max(image.width, image.height)), 1)
    if scale >= 1 { return image }
    let width = max(1, Int((Double(image.width) * scale).rounded()))
    let height = max(1, Int((Double(image.height) * scale).rounded()))
    let canvas = try drawingContext(width: width, height: height)
    canvas.interpolationQuality = .high
    canvas.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let resized = canvas.makeImage() else { throw SourceFailure.unavailable }
    return resized
}

private final class ResumeOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    func claim() -> Bool {
        lock.lock(); defer { lock.unlock() }
        if done { return false }
        done = true
        return true
    }
}

private func drawingContext(width: Int, height: Int) throws -> CGContext {
    guard width > 0, height > 0, let space = CGColorSpace(name: CGColorSpace.sRGB),
          let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { throw SourceFailure.unavailable }
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    return context
}

private func drawLabel(_ text: String, in context: CGContext, at frame: CGRect, cell: Int) {
    let size = max(14, Double(cell) / 9)
    let font = CTFontCreateWithName("Helvetica-Bold" as CFString, size, nil)
    let attributes: [NSAttributedString.Key: Any] = [NSAttributedString.Key(kCTFontAttributeName as String): font, NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(gray: 1, alpha: 1)]
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
    let bounds = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
    let padding = size / 3
    let box = CGRect(x: frame.minX + 4, y: frame.maxY - 4 - bounds.height - 2 * padding, width: bounds.width + 2 * padding, height: bounds.height + 2 * padding)
    context.setFillColor(CGColor(gray: 0, alpha: 0.72))
    context.addPath(CGPath(roundedRect: box, cornerWidth: padding, cornerHeight: padding, transform: nil))
    context.fillPath()
    context.textPosition = CGPoint(x: box.minX + padding - bounds.minX, y: box.minY + padding - bounds.minY)
    CTLineDraw(line, context)
}

private func encodedImage(_ image: CGImage) throws -> JSON {
    for quality in [0.82, 0.7, 0.55, 0.4] {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil) else { throw SourceFailure.unavailable }
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: quality, kCGImagePropertyOrientation: 1] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw SourceFailure.unavailable }
        if data.length <= photoImageByteLimit {
            return .object(["mime_type": .string("image/jpeg"), "width": .number(Double(image.width)), "height": .number(Double(image.height)), "bytes": .number(Double(data.length)), "image_base64": .string((data as Data).base64EncodedString())])
        }
    }
    throw SourceFailure.unavailable
}
