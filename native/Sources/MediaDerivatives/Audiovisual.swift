import Foundation
import AVFoundation
import Accelerate
import UniformTypeIdentifiers
import Darwin

extension MediaDerivatives {
    static func movieIdentifier(_ url: URL) async throws -> String? {
        guard UTType(filenameExtension: url.pathExtension)?.conforms(to: .movie) == true else { return nil }
        let asset = AVURLAsset(url: url)
        for item in try await asset.loadMetadata(for: .quickTimeMetadata) where item.identifier == .quickTimeMetadataContentIdentifier {
            return try await item.load(.stringValue)
        }
        return nil
    }
    static func audiovisual(_ input: URL, directory: URL) async throws -> MediaResult {
        let asset = AVURLAsset(url: input)
        if try await !asset.loadTracks(withMediaType: .video).isEmpty { return try await movie(input, directory: directory, name: "video") }
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else { throw MediaFailure.decode }
        let seconds = try await asset.load(.duration).seconds
        guard seconds.isFinite, seconds > 0 else { throw MediaFailure.decode }
        let file = directory.appendingPathComponent("audio.m4a")
        try await export(asset, preset: AVAssetExportPresetAppleM4A, file: file, type: .m4a)
        var result = MediaResult()
        result.media.duration_ms = Int((seconds * 1000).rounded())
        result.media.waveform = try waveform(asset, track: track, seconds: seconds)
        result.derivatives["audio"] = try variant(file, mime: "audio/mp4")
        return result
    }
    static func movie(_ input: URL, directory: URL, name: String) async throws -> MediaResult {
        let asset = AVURLAsset(url: input)
        let seconds = try await asset.load(.duration).seconds
        guard seconds.isFinite, seconds > 0 else { throw MediaFailure.decode }
        guard let original = try await asset.loadTracks(withMediaType: .video).first else { throw MediaFailure.decode }
        let originalSize = try await original.load(.naturalSize)
        let originalTransform = try await original.load(.preferredTransform)
        let oriented = originalSize.applying(originalTransform)
        var result = MediaResult()
        result.media.width = Int(abs(oriented.width).rounded())
        result.media.height = Int(abs(oriented.height).rounded())
        result.media.duration_ms = Int((seconds * 1000).rounded())
        let file = directory.appendingPathComponent(name + ".mp4")
        do { try await export(asset, preset: AVAssetExportPreset1920x1080, file: file, type: .mp4) }
        catch {
            try Task.checkCancellation()
            guard (error as NSError).domain == "AppleBridgeMediaExport" else { throw error }
            result.unavailable = [name: safeExportReason(error, input: input, directory: directory)]
            return result
        }
        let output = AVURLAsset(url: file)
        guard let track = try await output.loadTracks(withMediaType: .video).first else { throw MediaFailure.encode }
        let descriptions = try await track.load(.formatDescriptions)
        guard descriptions.allSatisfy({ CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_H264 }) else { throw MediaFailure.encode }
        let dimensions = try await track.load(.naturalSize)
        let transform = try await track.load(.preferredTransform)
        let size = dimensions.applying(transform)
        let width = Int(abs(size.width).rounded())
        let height = Int(abs(size.height).rounded())
        guard max(width, height) <= 1920, min(width, height) <= 1080 else { throw MediaFailure.encode }
        let generator = AVAssetImageGenerator(asset: output)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: 400, height: 400)
        let poster = try await generator.image(at: .zero).image
        let posterFile = directory.appendingPathComponent("poster.jpg")
        try write(poster, to: posterFile, alpha: false, quality: 0.8)
        result.derivatives["poster"] = try variant(posterFile, mime: "image/jpeg", width: poster.width, height: poster.height)
        generator.maximumSize = CGSize(width: 100, height: 100)
        result.media.thumbhash = try placeholder(try await generator.image(at: .zero).image)
        result.derivatives[name] = try variant(file, mime: "video/mp4", width: width, height: height)
        return result
    }
    @_spi(Testing) public static func export(_ asset: AVAsset, preset: String, file: URL, type: AVFileType) async throws {
        try Task.checkCancellation()
        let expected = type == .mp4 ? "mp4" : type == .m4a ? "m4a" : type == .mov ? "mov" : ""
        guard file.isFileURL, !expected.isEmpty, file.pathExtension.lowercased() == expected else { throw exportFailure("Output URL or extension does not match file type") }
        guard try await asset.load(.isExportable), await AVAssetExportSession.compatibility(ofExportPreset: preset, with: asset, outputFileType: type) else { throw exportFailure("Asset preset and file type are incompatible") }
        try Task.checkCancellation()
        let temporary = file.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + "." + expected)
        guard !FileManager.default.fileExists(atPath: temporary.path) else { throw exportFailure("Temporary export output already exists") }
        defer { try? FileManager.default.removeItem(at: temporary) }
        guard let session = AVAssetExportSession(asset: asset, presetName: preset), session.supportedFileTypes.contains(type) else { throw exportFailure("Export session does not support requested file type") }
        session.shouldOptimizeForNetworkUse = true
        let attempt = ExportAttempt(session)
        do {
            try await withTaskCancellationHandler {
                try Task.checkCancellation()
                try await withCheckedThrowingContinuation { continuation in attempt.start(temporary, type: type, continuation: continuation) }
            } onCancel: { attempt.cancel() }
        } catch {
            try Task.checkCancellation()
            if error is CancellationError { throw error }
            if (error as NSError).domain == "AppleBridgeMediaExport" { throw error }
            throw exportFailure((error as NSError).localizedDescription)
        }
        try Task.checkCancellation()
        guard Darwin.rename(temporary.path, file.path) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }
    static func waveform(_ asset: AVAsset, track: AVAssetTrack, seconds: Double) throws -> [UInt8] {
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 8000, AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true, AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false])
        guard reader.canAdd(output) else { throw MediaFailure.decode }
        reader.add(output)
        guard reader.startReading() else { throw MediaFailure.decode }
        defer { reader.cancelReading() }
        var peaks = [Float](repeating: 0, count: 64)
        var position = 0
        let samples = max(1, Int(ceil(seconds * 8000)))
        while let buffer = output.copyNextSampleBuffer() {
            try Task.checkCancellation()
            guard let block = CMSampleBufferGetDataBuffer(buffer) else { throw MediaFailure.decode }
            let count = CMBlockBufferGetDataLength(block) / MemoryLayout<Float>.size
            var values = [Float](repeating: 0, count: count)
            let status = values.withUnsafeMutableBytes { raw in CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: raw.count, destination: raw.baseAddress!) }
            guard status == kCMBlockBufferNoErr else { throw MediaFailure.decode }
            var offset = 0
            while offset < count {
                let bin = min(63, position * 64 / samples)
                let end = min(count, offset + max(1, (bin + 1) * samples / 64 - position))
                var peak: Float = 0
                values.withUnsafeBufferPointer { pointer in vDSP_maxmgv(pointer.baseAddress! + offset, 1, &peak, vDSP_Length(end - offset)) }
                peaks[bin] = max(peaks[bin], peak)
                position += end - offset
                offset = end
            }
        }
        guard reader.status == .completed else { throw MediaFailure.decode }
        return peaks.map { $0.isFinite ? UInt8((min(1, max(0, $0)) * 255).rounded()) : 0 }
    }
}
