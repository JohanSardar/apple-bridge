import Foundation
import AVFoundation
import CryptoKit
import MediaExportGuard
@_spi(Testing) import MediaDerivatives

struct ExportSafetyChecks {
    static func run() async throws {
        let tag = "ab-test-export-" + UUID().uuidString
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(tag)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("synthetic.mov")
        try await MediaChecks.movie(source, small: true, identifier: tag)
        let asset = AVURLAsset(url: source)
        try FileHandle.standardError.write(contentsOf: Data("export safety case: cancelled-before-start\n".utf8))
        let cancelled = Task<Void, Error> {
            withUnsafeCurrentTask { $0?.cancel() }
            try await MediaDerivatives.export(asset, preset: AVAssetExportPreset1920x1080, file: directory.appendingPathComponent("cancelled.mp4"), type: .mp4)
        }
        var cancellationRejected = false
        do { try await cancelled.value } catch is CancellationError { cancellationRejected = true }
        guard cancellationRejected, !FileManager.default.fileExists(atPath: directory.appendingPathComponent("cancelled.mp4").path) else { throw CheckFailure.mismatch }
        guard try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil).count == 1 else { throw CheckFailure.mismatch }
        let digest = SHA256.hash(data: try Data(contentsOf: source)).map { String(format: "%02x", $0) }.joined()
        let cache = MediaDerivatives.root.appendingPathComponent(digest)
        guard !FileManager.default.fileExists(atPath: cache.path) else { throw CheckFailure.mismatch }
        defer { try? FileManager.default.removeItem(at: cache) }
        let cancelledItem = Task<MediaResult, Error> {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await MediaDerivatives.make(source, digest: digest)
        }
        do { _ = try await cancelledItem.value; throw CheckFailure.mismatch } catch is CancellationError {}
        guard MediaDerivatives.cached(digest) == nil else { throw CheckFailure.mismatch }
        let retried = try await MediaDerivatives.make(source, digest: digest)
        guard retried.derivatives["video"] != nil, retried.unavailable == nil else { throw CheckFailure.mismatch }
        let trapped = ThrowingExportSession(asset: asset, presetName: AVAssetExportPreset1920x1080)!
        let exception = ABMediaExportGuard.start(trapped, outputURL: directory.appendingPathComponent("guard.mp4"), fileType: .mp4, completion: { _ in })
        let details = exception.map { ($0 as NSError).userInfo } ?? [:]
        guard details["exception_name"] as? String == "SyntheticExportException", details["reason"] as? String == "Synthetic export configuration failure" else { throw CheckFailure.mismatch }
        try FileHandle.standardError.write(contentsOf: Data("export safety case: incompatible-preset-type\n".utf8))
        let preset = AVAssetExportPresetAppleProRes422LPCM
        let incompatible = await AVAssetExportSession.compatibility(ofExportPreset: preset, with: asset, outputFileType: .mp4)
        guard !incompatible else { throw CheckFailure.mismatch }
        let output = directory.appendingPathComponent("unsupported.mp4")
        var rejected = false
        do {
            try await MediaDerivatives.export(asset, preset: preset, file: output, type: .mp4)
        } catch {
            let failure = error as NSError
            rejected = failure.domain == "AppleBridgeMediaExport" && !(failure.userInfo["reason"] as? String ?? "").isEmpty
        }
        guard rejected, !FileManager.default.fileExists(atPath: output.path) else { throw CheckFailure.mismatch }
        let valid = directory.appendingPathComponent("valid.mp4")
        try await MediaDerivatives.export(asset, preset: AVAssetExportPreset1920x1080, file: valid, type: .mp4)
        guard FileManager.default.fileExists(atPath: valid.path) else { throw CheckFailure.mismatch }
        print("synthetic incompatible export rejected; following export completed")
    }
}

private final class ThrowingExportSession: AVAssetExportSession, @unchecked Sendable {
    override var outputURL: URL? {
        get { nil }
        set { NSException(name: NSExceptionName("SyntheticExportException"), reason: "Synthetic export configuration failure").raise() }
    }
}
