import Foundation
import Darwin
import AVFoundation
import CryptoKit
import ImageIO
import UniformTypeIdentifiers
import MediaDerivatives

struct ExportOverlapChecks {
    static func worker() async throws {
        let args = CommandLine.arguments
        let source = URL(fileURLWithPath: args[2]), movie = URL(fileURLWithPath: args[3]), directory = URL(fileURLWithPath: args[5]), label = args[6]
        try Data().write(to: directory.appendingPathComponent(label + ".started"))
        let result = try await MediaDerivatives.make(source, digest: args[4], companions: [movie]) { _ in
            try Data().write(to: directory.appendingPathComponent(label + ".ready"))
            while !FileManager.default.fileExists(atPath: directory.appendingPathComponent("release").path) { try await Task.sleep(for: .milliseconds(10)) }
        }
        try JSONEncoder().encode(result).write(to: directory.appendingPathComponent(label + ".done"))
    }
    static func wait(_ seconds: Double, _ condition: () -> Bool) async throws -> Bool {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            if condition() { return true }
            try await Task.sleep(for: .milliseconds(10))
        }
        return condition()
    }
    static func run() async throws {
        let identifier = UUID().uuidString
        let tag = "ab-test-export-overlap-" + identifier
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(tag)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("synthetic.heic"), movie = directory.appendingPathComponent("synthetic.mov")
        let context = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(CGColor(red: 0.2, green: 0.6, blue: 0.8, alpha: 1));context.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
        let output = CGImageDestinationCreateWithURL(source as CFURL, UTType.heic.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(output, context.makeImage()!, [kCGImagePropertyMakerAppleDictionary: ["17": identifier]] as CFDictionary)
        guard CGImageDestinationFinalize(output) else { throw ExportOverlapFailure.checkpoint("fixture") }
        try await MediaChecks.movie(movie, small: true, identifier: identifier)
        guard MediaDerivatives.liveIdentifier(source) == identifier else { throw ExportOverlapFailure.checkpoint("still-identifier") }
        let metadata = try await AVURLAsset(url: movie).loadMetadata(for: .quickTimeMetadata)
        var movieIdentifier: String?
        for item in metadata where item.identifier == .quickTimeMetadataContentIdentifier { movieIdentifier = try await item.load(.stringValue) }
        guard movieIdentifier == identifier else { throw ExportOverlapFailure.checkpoint("movie-identifier") }
        let digest = SHA256.hash(data: try Data(contentsOf: source)).map { String(format: "%02x", $0) }.joined()
        let cache = MediaDerivatives.root.appendingPathComponent(digest)
        guard !FileManager.default.fileExists(atPath: cache.path) else { throw ExportOverlapFailure.checkpoint("fresh-cache") }
        defer { try? FileManager.default.removeItem(at: cache) }
        let first = Process(), second = Process()
        for (process, label) in [(first, "first"), (second, "second")] {
            process.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
            process.arguments = ["export-worker", source.path, movie.path, digest, directory.path, label]
        }
        defer {
            try? Data().write(to: directory.appendingPathComponent("release"))
            for process in [first, second] where process.isRunning { process.terminate();process.waitUntilExit() }
        }
        try first.run()
        guard try await wait(5, { FileManager.default.fileExists(atPath: directory.appendingPathComponent("first.ready").path) }) else { throw ExportOverlapFailure.checkpoint("first-ready") }
        try second.run()
        guard try await wait(5, { FileManager.default.fileExists(atPath: directory.appendingPathComponent("second.started").path) }) else { throw ExportOverlapFailure.checkpoint("second-started") }
        let duplicate = try await wait(0.5, { FileManager.default.fileExists(atPath: directory.appendingPathComponent("second.ready").path) })
        guard !duplicate else {
            try FileHandle.standardError.write(contentsOf: Data("same-digest work overlapped across processes\n".utf8))
            throw ExportOverlapFailure.checkpoint("duplicate-work")
        }
        try Data().write(to: directory.appendingPathComponent("release"))
        guard try await wait(15, { !first.isRunning && !second.isRunning }), first.terminationStatus == 0, second.terminationStatus == 0 else { throw ExportOverlapFailure.checkpoint("both-exited") }
        let a = try JSONDecoder().decode(MediaResult.self, from: Data(contentsOf: directory.appendingPathComponent("first.done")))
        let b = try JSONDecoder().decode(MediaResult.self, from: Data(contentsOf: directory.appendingPathComponent("second.done")))
        guard let live = a.derivatives["live"], live.digest == b.derivatives["live"]?.digest, !FileManager.default.fileExists(atPath: directory.appendingPathComponent("second.ready").path) else {
            let details: [String: Any] = ["first_live": a.derivatives["live"] != nil, "second_live": b.derivatives["live"] != nil, "first_awaiting": a.awaitingLive == true, "second_awaiting": b.awaitingLive == true, "second_progress": FileManager.default.fileExists(atPath: directory.appendingPathComponent("second.ready").path), "cached": MediaDerivatives.cached(digest) != nil]
            try FileHandle.standardError.write(contentsOf: JSONSerialization.data(withJSONObject: details, options: [.sortedKeys]))
            throw ExportOverlapFailure.checkpoint("cache-reused")
        }
        let abandoned = cache.appendingPathComponent(UUID().uuidString + ".mp4")
        let empty = cache.appendingPathComponent(UUID().uuidString + ".m4a")
        try Data([0, 1, 2, 3]).write(to: abandoned)
        try Data().write(to: empty)
        guard MediaDerivatives.cached(digest) != nil, !FileManager.default.fileExists(atPath: abandoned.path), !FileManager.default.fileExists(atPath: empty.path) else { throw ExportOverlapFailure.checkpoint("cached-abandoned-output-cleanup") }
        try FileManager.default.removeItem(at: cache)
        for label in ["first", "second"] {
            for suffix in ["started", "ready", "done"] { try? FileManager.default.removeItem(at: directory.appendingPathComponent(label + "." + suffix)) }
        }
        try FileManager.default.removeItem(at: directory.appendingPathComponent("release"))
        let doomed = Process(), successor = Process()
        for (process, label) in [(doomed, "first"), (successor, "second")] {
            process.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
            process.arguments = ["export-worker", source.path, movie.path, digest, directory.path, label]
        }
        defer { for process in [doomed, successor] where process.isRunning { process.terminate(); process.waitUntilExit() } }
        try doomed.run()
        guard try await wait(5, { FileManager.default.fileExists(atPath: directory.appendingPathComponent("first.ready").path) }) else { throw ExportOverlapFailure.checkpoint("doomed-ready") }
        try Data([0, 1, 2, 3]).write(to: abandoned)
        try Data().write(to: empty)
        guard MediaDerivatives.cached(digest) != nil, FileManager.default.fileExists(atPath: abandoned.path), FileManager.default.fileExists(atPath: empty.path) else { throw ExportOverlapFailure.checkpoint("active-output-preserved") }
        Darwin.kill(doomed.processIdentifier, SIGKILL)
        doomed.waitUntilExit()
        try Data().write(to: directory.appendingPathComponent("release"))
        try successor.run()
        guard try await wait(15, { !successor.isRunning }), successor.terminationStatus == 0 else { throw ExportOverlapFailure.checkpoint("successor-exited") }
        let recovered = try JSONDecoder().decode(MediaResult.self, from: Data(contentsOf: directory.appendingPathComponent("second.done")))
        guard recovered.derivatives["live"] != nil else { throw ExportOverlapFailure.checkpoint("successor-live") }
        guard !FileManager.default.fileExists(atPath: abandoned.path), !FileManager.default.fileExists(atPath: empty.path) else { throw ExportOverlapFailure.checkpoint("dead-holder-output-cleanup") }
        print("same-digest work serialized across processes; cached result reused")
        print("process death released digest lease; successor completed")
    }
}

private enum ExportOverlapFailure: Error { case checkpoint(String) }
