#if LAB
import Foundation
import AppKit
import Darwin
@preconcurrency import Photos
import CryptoKit
import PhotosSource
import SourceSupport

private actor PhotosProofSink: ObservationSink {
    var checkpoints = 0
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard source == "photos", completionEvidence == "complete" else { throw SourceFailure.corrupt }
        checkpoints += 1
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.corrupt }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
}

@MainActor
enum PhotosRealProof {
    static func run(_ scenario: String) async throws -> JSON {
        guard ["PHO-READ-REAL", "PHO-IMPORT-REAL", "PHO-ORIGINAL-REAL", "PHO-IDLE"].contains(scenario) else { throw SourceFailure.invalid }
        guard PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized else {
            return .object(["grant_passed": .bool(false)])
        }
        let source = PhotosSource()
        func command(_ op: String, _ args: JSON) async throws -> JSON {
            try await source.dispatch(source.prepare(.object(["action": .object(["op": .string(op), "args": args])])))
        }
        let title = "Bridge Test " + UUID().uuidString
        let created = try await command("photos.create_album", .object(["name": .string(title)]))
        let albumID = created["native_key"].string
        guard let album = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [albumID], options: nil).firstObject, album.localizedTitle == title else { throw SourceFailure.corrupt }
        func cleanup() throws {
            let assets = PHAsset.fetchAssets(in: album, options: nil)
            try PHPhotoLibrary.shared().performChangesAndWait {
                PHAssetChangeRequest.deleteAssets(assets)
                PHAssetCollectionChangeRequest.deleteAssetCollections([album] as NSArray)
            }
            guard PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [albumID], options: nil).count == 0 else { throw SourceFailure.corrupt }
        }
        do {
            guard let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
            for x in 0..<2 { for y in 0..<2 { image.setColor(.red, atX: x, y: y) } }
            guard let bytes = image.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
            let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
            let imported = try await command("photos.import", .object(["data": .string(bytes.base64EncodedString()), "digest": .string(digest), "kind": .string("image"), "album": .string(albumID)]))
            let assetID = imported["native_key"].string
            let rows = try await source.search(.object(["album": .string(albumID)]))["items"].array
            guard rows.count == 1, rows[0]["local_id"].string == assetID else { throw SourceFailure.corrupt }
            let original = try await source.blob("photos.original", .object(["asset": .string(assetID)]))
            guard original["digest"].string == digest, original["size"].number == Double(bytes.count) else { throw SourceFailure.corrupt }
            let read = try await source.readBlob(original["handle"].string, offset: 0, length: 131072)
            guard read["data"].string == bytes.base64EncodedString(), read["eof"].bool else { throw SourceFailure.corrupt }
            var result: [String: JSON] = ["grant_passed": .bool(true), "read_passed": .bool(true), "import_passed": .bool(true), "original_passed": .bool(true), "asset_count": .number(1), "original_digest": .string(digest)]
            if scenario == "PHO-IDLE" {
                let sink = PhotosProofSink()
                let capture = Task { try await source.capture(.null, into: sink) }
                defer { capture.cancel() }
                let deadline = Date().addingTimeInterval(30)
                while await sink.checkpoints == 0 && Date() < deadline { try await Task.sleep(for: .milliseconds(50)) }
                guard await sink.checkpoints == 1 else { throw SourceFailure.corrupt }
                let cpuStart = clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID)
                let start = Date()
                try await Task.sleep(for: .seconds(300))
                let cpu = Double(clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID) - cpuStart) / 1e9 / Date().timeIntervalSince(start) * 100
                result["idle_passed"] = .bool(cpu < 2)
            }
            try cleanup()
            result["cleanup_passed"] = .bool(true)
            return .object(result)
        } catch {
            try cleanup()
            throw error
        }
    }
}
#endif
