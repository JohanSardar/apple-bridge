import Foundation
import Darwin
import AVFoundation
import CryptoKit
import ImageIO
import UniformTypeIdentifiers
import MessagesSource
import MediaDerivatives
import SourceSupport

actor YieldSink: ObservationSink {
    var partials = 0
    var complete = false
    var motionComplete = false
    var gates: [CheckedContinuation<Void, Never>] = []
    var released = false
    var finalDigest = ""
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        if observations.contains(where: { $0["native_key"].string == "synthetic-yield-motion" && !$0["data"]["derivatives"]["video"]["digest"].string.isEmpty }) { motionComplete = true }
        for item in observations where item["native_key"].string == "synthetic-yield-attachment" && !item["data"]["derivatives"]["thumb"]["digest"].string.isEmpty {
            if !item["data"]["derivatives"]["live"]["digest"].string.isEmpty { complete = true; finalDigest = item["data"]["blob"]["digest"].string }
            else {
                partials += 1
                if !released { await withCheckedContinuation { gates.append($0) } }
            }
        }
    }
    func release(final: Bool = false) {
        released = final
        let ready = gates
        gates.removeAll()
        for gate in ready { gate.resume() }
    }
    func count() -> Int { partials }
    func digest() -> String { finalDigest }
    func finished() -> Bool { complete && motionComplete }
}

struct MediaYieldChecks {
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
        try MessagesCaptureChecks.require(writer.startWriting(), "synthetic motion writer")
        writer.startSession(atSourceTime: .zero)
        for index in 0..<12 {
            while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(5)) }
            var pixel: CVPixelBuffer?
            try MessagesCaptureChecks.require(CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &pixel) == kCVReturnSuccess, "synthetic motion buffer")
            guard let pixel else { throw SourceFailure.invalid }
            CVPixelBufferLockBaseAddress(pixel, [])
            memset(CVPixelBufferGetBaseAddress(pixel)!, 120, CVPixelBufferGetBytesPerRow(pixel) * 32)
            CVPixelBufferUnlockBaseAddress(pixel, [])
            try MessagesCaptureChecks.require(adaptor.append(pixel, withPresentationTime: CMTime(value: Int64(index), timescale: 12)), "synthetic motion frame")
        }
        input.markAsFinished()
        writer.endSession(atSourceTime: CMTime(value: 1, timescale: 1))
        await writer.finishWriting()
        try MessagesCaptureChecks.require(writer.status == .completed, "synthetic motion completed")
    }
    static func run(change: Bool = false, delayed: Bool = false, metadata: Bool = false) async throws {
        let tag = "ab-test-yield-" + UUID().uuidString
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(tag)
        let attachments = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/Attachments/" + tag)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        try FileManager.default.createDirectory(at: attachments, withIntermediateDirectories: false)
        let image = attachments.appendingPathComponent("still.heic"), path = root.appendingPathComponent("chat.db").path
        var caches: [URL] = []
        defer {
            for cache in caches { try? FileManager.default.removeItem(at: cache) }
            let identity = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
            for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: NSHomeDirectory() + "/Library/Application Support/AppleBridge/messages/" + identity + ".sqlite" + suffix) }
            try? FileManager.default.removeItem(at: attachments)
            try? FileManager.default.removeItem(at: root)
        }
        let context = CGContext(data: nil, width: 1600, height: 800, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(CGColor(red: 0.2, green: 0.6, blue: 0.9, alpha: 1));context.fill(CGRect(x: 0, y: 0, width: 1600, height: 800))
        let destination = CGImageDestinationCreateWithURL(image as CFURL, UTType.heic.identifier as CFString, 1, nil)!
        let identifier = UUID().uuidString
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyMakerAppleDictionary: ["17": identifier]] as CFDictionary)
        try MessagesCaptureChecks.require(CGImageDestinationFinalize(destination), "synthetic Live still")
        let digest = SHA256.hash(data: try Data(contentsOf: image)).map { String(format: "%02x", $0) }.joined()
        let directory = MediaDerivatives.root.appendingPathComponent(digest)
        try MessagesCaptureChecks.require(!FileManager.default.fileExists(atPath: directory.path), "unique yield fixture")
        caches.append(directory)
        var expectedDigest = digest
        let movie = attachments.appendingPathComponent("motion.mov")
        try await motion(movie, identifier: identifier)
        let movieDigest = SHA256.hash(data: try Data(contentsOf: movie)).map { String(format: "%02x", $0) }.joined()
        let movieCache = MediaDerivatives.root.appendingPathComponent(movieDigest)
        try MessagesCaptureChecks.require(!FileManager.default.fileExists(atPath: movieCache.path), "unique motion fixture")
        caches.append(movieCache)
        let db = try SyntheticDatabase(path)
        let filename = image.path.replacingOccurrences(of: "'", with: "''")
        try db.execute("PRAGMA journal_mode=WAL; CREATE TABLE message(guid TEXT,text TEXT); INSERT INTO message VALUES('synthetic-yield-message','synthetic'); CREATE TABLE attachment(guid TEXT,filename TEXT,mime_type TEXT,uti TEXT,total_bytes INTEGER,transfer_state INTEGER); INSERT INTO attachment VALUES('synthetic-yield-attachment','" + filename + "','image/heic','public.heic',0,0); CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER); INSERT INTO message_attachment_join VALUES(1,1);")
        if !delayed { try db.execute("INSERT INTO attachment VALUES('synthetic-yield-motion','" + movie.path.replacingOccurrences(of: "'", with: "''") + "','video/quicktime','com.apple.quicktime-movie',0,0);") }
        let source = Messages(path: path, diagnostics: root.appendingPathComponent("diagnostics.json"), labCommands: true), sink = YieldSink()
        let capture = Task { try await source.capture(.null, into: sink) }
        defer { capture.cancel() }
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait(15) { await sink.count() > 0 }, "completed image stage before burst")
        for index in 0..<6 {
            let pulse = ForegroundActivity.shared.begin()
            if metadata && index == 2 {
                let before = try FileManager.default.attributesOfItem(atPath: image.path)
                let value: [UInt8] = [1]
                let status = value.withUnsafeBytes { Darwin.setxattr(image.path, "com.applebridge.synthetic-yield", $0.baseAddress, $0.count, 0, 0) }
                try MessagesCaptureChecks.require(status == 0, "set synthetic xattr during suspended work")
                let after = try FileManager.default.attributesOfItem(atPath: image.path)
                try MessagesCaptureChecks.require(before[.modificationDate] as? Date == after[.modificationDate] as? Date, "xattr preserves source mtime")
                let current = SHA256.hash(data: try Data(contentsOf: image)).map { String(format: "%02x", $0) }.joined()
                try MessagesCaptureChecks.require(current == digest, "xattr preserves source bytes")
            }
            if change && index == 2 {
                context.setFillColor(CGColor(red: 0.9, green: 0.1, blue: 0.2, alpha: 1))
                context.fill(CGRect(x: 0, y: 0, width: 1600, height: 800))
                let replacement = CGImageDestinationCreateWithURL(image as CFURL, UTType.heic.identifier as CFString, 1, nil)!
                CGImageDestinationAddImage(replacement, context.makeImage()!, [kCGImagePropertyMakerAppleDictionary: ["17": identifier]] as CFDictionary)
                try MessagesCaptureChecks.require(CGImageDestinationFinalize(replacement), "replace synthetic source bytes")
                let updated = try Data(contentsOf: image)
                expectedDigest = SHA256.hash(data: updated).map { String(format: "%02x", $0) }.joined()
                let changedCache = MediaDerivatives.root.appendingPathComponent(expectedDigest)
                try MessagesCaptureChecks.require(expectedDigest != digest && !FileManager.default.fileExists(atPath: changedCache.path), "changed source has a fresh digest")
                caches.append(changedCache)
                try db.execute("UPDATE attachment SET total_bytes=" + String(updated.count) + " WHERE guid='synthetic-yield-attachment';")
            }
            await sink.release()
            try await Task.sleep(for: .milliseconds(20))
            ForegroundActivity.shared.end(pulse)
            try await Task.sleep(for: .milliseconds(110))
        }
        await sink.release(final: true)
        if delayed {
            try await Task.sleep(for: .milliseconds(2500))
            try MessagesCaptureChecks.require(await sink.count() == 1, "missing companion does not recompute the still")
            try MessagesCaptureChecks.require(MediaDerivatives.cached(digest)?.awaitingLive == true, "missing companion remains retryable")
            try db.execute("INSERT INTO attachment VALUES('synthetic-yield-motion','" + movie.path.replacingOccurrences(of: "'", with: "''") + "','video/quicktime','com.apple.quicktime-movie',0,0);")
        }
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait(15) { await sink.finished() }, "Live export completes after burst")
        capture.cancel()
        _ = try? await capture.value
        let bytes = try Data(contentsOf: root.appendingPathComponent("media-work-trace.jsonl"))
        let rows = bytes.split(separator: 10).compactMap { try? JSONDecoder().decode(JSON.self, from: Data($0)) }
        let starts = rows.filter { $0["stage"].string == "media_started" }.count
        let computes = rows.filter { $0["stage"].string == "media_compute_started" }.count
        let partials = await sink.count()
        let startsByDigest = Dictionary(grouping: rows.filter { $0["stage"].string == "media_started" }, by: { $0["keys"].array.first?.string ?? "" }).mapValues(\.count)
        let stages = rows.filter { $0["stage"].string == "media_stage_image_display" }.count
        let metrics: [String: Any] = ["mode": metadata ? "metadata_only" : change ? "changed_source" : delayed ? "delayed_companion" : "burst", "starts": starts, "computes": computes, "image_stages": stages, "partial_publications": partials, "maximum_starts_per_digest": startsByDigest.values.max() ?? 0]
        var encoded = try JSONSerialization.data(withJSONObject: metrics, options: [.sortedKeys])
        encoded.append(10)
        try FileHandle.standardError.write(contentsOf: encoded)
        if !change && !delayed { try MessagesCaptureChecks.require(starts <= 3, "six foreground pulses start at most three media attempts") }
        try MessagesCaptureChecks.require(startsByDigest.values.allSatisfy { $0 <= 3 }, "six foreground pulses start at most three media attempts per digest")
        try MessagesCaptureChecks.require(computes == (change || delayed ? 3 : 2) && partials == (change ? 2 : 1) && stages == (change ? 2 : 1), "completed image stages are reused by digest")
        try MessagesCaptureChecks.require(MediaDerivatives.cached(expectedDigest)?.derivatives["live"] != nil, "final Live derivative is persisted")
        try MessagesCaptureChecks.require(await sink.digest() == expectedDigest, "final publication uses current source digest")
        print("cooperative media burst checks passed")
    }
}
