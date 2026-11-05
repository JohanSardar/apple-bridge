import Foundation
import AVFoundation
import CryptoKit
@_spi(Testing) import MediaDerivatives

private final class ExportYieldRace: @unchecked Sendable {
    private let lock = NSLock()
    private var attempts = 0
    func interrupt() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        attempts += 1
        return attempts <= 2
    }
    func count() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return attempts
    }
}

struct MediaExportYieldChecks {
    private static func require(_ condition: Bool, _ name: String) throws {
        if !condition { print("failed: " + name); throw CheckFailure.mismatch }
    }
    static func motion(_ file: URL, identifier: String) async throws {
        let writer = try AVAssetWriter(outputURL: file, fileType: .mov)
        let metadata = AVMutableMetadataItem()
        metadata.identifier = .quickTimeMetadataContentIdentifier
        metadata.value = identifier as NSString
        metadata.dataType = kCMMetadataBaseDataType_UTF8 as String
        writer.metadata = [metadata]
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 32, AVVideoHeightKey: 32])
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 32, kCVPixelBufferHeightKey as String: 32])
        writer.add(input)
        try require(writer.startWriting(), "synthetic motion writer")
        writer.startSession(atSourceTime: .zero)
        for index in 0..<12 {
            while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(5)) }
            var pixel: CVPixelBuffer?
            try require(CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &pixel) == kCVReturnSuccess, "synthetic motion buffer")
            guard let pixel else { throw CheckFailure.mismatch }
            CVPixelBufferLockBaseAddress(pixel, [])
            memset(CVPixelBufferGetBaseAddress(pixel)!, 120, CVPixelBufferGetBytesPerRow(pixel) * 32)
            CVPixelBufferUnlockBaseAddress(pixel, [])
            try require(adaptor.append(pixel, withPresentationTime: CMTime(value: Int64(index), timescale: 12)), "synthetic motion frame")
        }
        input.markAsFinished()
        writer.endSession(atSourceTime: CMTime(value: 1, timescale: 1))
        await writer.finishWriting()
        try require(writer.status == .completed, "synthetic motion completed")
    }
    static func run() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-export-yield-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let input = directory.appendingPathComponent("source.mov"), output = directory.appendingPathComponent("result.mp4")
        try await motion(input, identifier: UUID().uuidString)
        let digest = SHA256.hash(data: try Data(contentsOf: input)).map { String(format: "%02x", $0) }.joined()
        let race = ExportYieldRace()
        let work = MediaWork(digest: digest, checkpoint: { _ in try Task.checkCancellation() }, interruption: { callback in
            if race.interrupt() { callback() }
            return {}
        }, trace: { _ in })
        try await MediaWork.$current.withValue(work) {
            try await MediaDerivatives.export(AVURLAsset(url: input), preset: AVAssetExportPreset1920x1080, file: output, type: .mp4)
        }
        try require(race.count() == 3, "two cancel-before-start yields resume with a fresh export attempt")
        let tracks = try await AVURLAsset(url: output).loadTracks(withMediaType: .video)
        try require(tracks.count == 1 && !Task.isCancelled, "export yield preserves the parent task and produces playable output")
        let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey])
        try require(files.count == 2 && files.allSatisfy { ((try? $0.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) > 0 }, "yielded export attempts leave no temporary or partial files")
        print("cooperative media export checks passed")
    }
}
