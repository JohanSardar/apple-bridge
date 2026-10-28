import Foundation
import AppKit
@preconcurrency import Photos
import SourceSupport

final class PhotoDataRequest: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Data, any Error>?
    private var finished = false
    private var overflow = false
    private var cancellation: (@Sendable () -> Void)?
    private var data = Data()
    func bind(_ value: CheckedContinuation<Data, any Error>) {
        lock.lock()
        if finished { lock.unlock(); value.resume(throwing: CancellationError()); return }
        continuation = value
        lock.unlock()
    }
    func attach(_ cancel: @escaping @Sendable () -> Void) {
        lock.lock()
        if finished { lock.unlock(); cancel(); return }
        cancellation = cancel
        lock.unlock()
    }
    func append(_ chunk: Data) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        guard chunk.count <= (16 << 20) - data.count else { overflow = true; lock.unlock(); fail(SourceFailure.unavailable); return }
        data.append(chunk)
        lock.unlock()
    }
    func complete(_ supplied: Data? = nil) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        let result = supplied ?? data
        guard !overflow, !result.isEmpty, result.count <= 16 << 20 else { lock.unlock(); fail(SourceFailure.unavailable); return }
        finished = true
        let value = continuation
        continuation = nil
        cancellation = nil
        data = Data()
        lock.unlock()
        value?.resume(returning: result)
    }
    func fail(_ error: any Error) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        finished = true
        let value = continuation
        let cancel = cancellation
        continuation = nil
        cancellation = nil
        data = Data()
        lock.unlock()
        cancel?()
        value?.resume(throwing: error)
    }
}

func originalPhotoData(_ asset: PHAsset) async throws -> Data {
    let resources = PHAssetResource.assetResources(for: asset)
    let type: PHAssetResourceType = asset.mediaType == .video ? .video : .photo
    guard let resource = resources.first(where: { $0.type == type }) else { throw SourceFailure.unavailable }
    let request = PhotoDataRequest()
    return try await withTaskCancellationHandler {
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            request.bind(continuation)
            let options = PHAssetResourceRequestOptions()
            options.isNetworkAccessAllowed = true
            let manager = PHAssetResourceManager.default()
            let identifier = manager.requestData(for: resource, options: options, dataReceivedHandler: { request.append($0) }, completionHandler: { error in
                if error != nil { request.fail(SourceFailure.unavailable) } else { request.complete() }
            })
            request.attach { manager.cancelDataRequest(identifier) }
        }
    } onCancel: { request.fail(CancellationError()) }
}

func thumbnailPhotoData(_ asset: PHAsset) async throws -> Data {
    let request = PhotoDataRequest()
    return try await withTaskCancellationHandler {
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            request.bind(continuation)
            let manager = PHImageManager.default()
            let options = PHImageRequestOptions()
            options.isNetworkAccessAllowed = true
            options.deliveryMode = .highQualityFormat
            options.resizeMode = .exact
            let identifier = manager.requestImage(for: asset, targetSize: CGSize(width: 256, height: 256), contentMode: .aspectFit, options: options) { image, info in
                if (info?[PHImageResultIsDegradedKey] as? Bool) == true { return }
                guard let image, let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil), let data = NSBitmapImageRep(cgImage: cg).representation(using: .jpeg, properties: [.compressionFactor: 0.85]) else { request.fail(SourceFailure.unavailable); return }
                request.complete(data)
            }
            request.attach { manager.cancelImageRequest(identifier) }
        }
    } onCancel: { request.fail(CancellationError()) }
}
