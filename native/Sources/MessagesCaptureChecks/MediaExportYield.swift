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
    static func run() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-export-yield-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let input = directory.appendingPathComponent("source.mov"), output = directory.appendingPathComponent("result.mp4")
        try await MediaYieldChecks.motion(input, identifier: UUID().uuidString)
        let digest = SHA256.hash(data: try Data(contentsOf: input)).map { String(format: "%02x", $0) }.joined()
        let race = ExportYieldRace()
        let work = MediaWork(digest: digest, checkpoint: { _ in try Task.checkCancellation() }, interruption: { callback in
            if race.interrupt() { callback() }
            return {}
        }, trace: { _ in })
        try await MediaWork.$current.withValue(work) {
            try await MediaDerivatives.export(AVURLAsset(url: input), preset: AVAssetExportPreset1920x1080, file: output, type: .mp4)
        }
        try MessagesCaptureChecks.require(race.count() == 3, "two cancel-before-start yields resume with a fresh export attempt")
        let tracks = try await AVURLAsset(url: output).loadTracks(withMediaType: .video)
        try MessagesCaptureChecks.require(tracks.count == 1 && !Task.isCancelled, "export yield preserves the parent task and produces playable output")
        let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey])
        try MessagesCaptureChecks.require(files.count == 2 && files.allSatisfy { ((try? $0.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) > 0 }, "yielded export attempts leave no temporary or partial files")
        print("cooperative media export checks passed")
    }
}
