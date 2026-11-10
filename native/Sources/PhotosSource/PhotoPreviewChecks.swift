#if LAB
import Foundation
import CoreGraphics
import SourceSupport

extension PhotosChecks {
    static func testPreviewChecks() throws {
        try testImageRequestCancellationAlwaysResumesOnce()
        try testLargestImageResultFitsNativeFrame()
    }

    private static func outcome(_ body: @escaping @Sendable () async throws -> CGImage) -> Result<CGImage, Error> {
        let done = DispatchSemaphore(value: 0)
        let box = OutcomeBox()
        Task {
            do { box.value = .success(try await body()) } catch { box.value = .failure(error) }
            done.signal()
        }
        done.wait()
        return box.value ?? .failure(SourceFailure.corrupt)
    }

    private static func cancelled(_ result: Result<CGImage, Error>) -> Bool {
        if case .failure(let error) = result { return error is CancellationError }
        return false
    }

    static func testImageRequestCancellationAlwaysResumesOnce() throws {
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue),
              let pixel = context.makeImage() else { throw SourceFailure.corrupt }
        let early = ImageRequest()
        early.cancel()
        try require(cancelled(outcome { try await withCheckedThrowingContinuation { continuation in _ = early.begin(continuation) } }))

        let silent = ImageRequest()
        let silentResult = outcome {
            try await withCheckedThrowingContinuation { continuation in
                guard silent.begin(continuation) else { return }
                DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) { silent.cancel() }
            }
        }
        try require(cancelled(silentResult))
        silent.finish(.success(pixel))
        silent.cancel()

        let delivered = ImageRequest()
        let image = outcome {
            try await withCheckedThrowingContinuation { continuation in
                guard delivered.begin(continuation) else { return }
                delivered.finish(.success(pixel))
                delivered.finish(.failure(SourceFailure.unavailable))
            }
        }
        delivered.cancel()
        try require((try? image.get())?.width == 1)
    }

    static func testLargestImageResultFitsNativeFrame() throws {
        let worst = String(repeating: "/", count: (photoImageByteLimit + 2) / 3 * 4 / 64) + String(repeating: "A", count: (photoImageByteLimit + 2) / 3 * 4 * 63 / 64)
        let cells = (1...30).map { index in JSON.object(["number": .number(Double(index)), "asset_id": .string(String(repeating: "F", count: 36) + "/L0/001"), "type": .string("video"), "created_at": .string("2026-11-09T21:30:55.315Z"), "rendered": .bool(true)]) }
        let sheet = JSON.object(["mime_type": .string("image/jpeg"), "width": .number(3104), "height": .number(1564), "bytes": .number(Double(photoImageByteLimit)), "image_base64": .string(worst), "cells": .array(cells), "columns": .number(8)])
        _ = try fitsNativeFrame(sheet)
        try require(photoResultByteLimit + 4096 < 262_144)
        try rejects(try fitsNativeFrame(.object(["image_base64": .string(String(repeating: "A", count: photoResultByteLimit))])))
    }
}

private final class OutcomeBox: @unchecked Sendable {
    var value: Result<CGImage, Error>?
}
#endif
