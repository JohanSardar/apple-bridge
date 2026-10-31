#if LAB
import Foundation
import AppKit
import Darwin
@preconcurrency import Photos
import CryptoKit
import PhotosSource
import SourceSupport

@MainActor
enum PhotosRealProof {
    static func run(_ scenario: String) async throws -> JSON {
        guard ["PHO-READ-REAL", "PHO-IMPORT-REAL", "PHO-ORIGINAL-REAL", "PHO-IDLE"].contains(scenario) else { throw SourceFailure.invalid }
        guard PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized else {
            return .object(["grant_passed": .bool(false)])
        }
        let source = PhotosSource()
        if scenario == "PHO-IDLE" {
            let readiness = await source.open()
            return .object(["grant_passed": .bool(true), "read_passed": .bool(readiness.state == "ready"), "asset_count": .number(Double(PHAsset.fetchAssets(with: nil).count))])
        }
        func command(_ op: String, _ args: JSON) async throws -> JSON {
            try await source.dispatch(source.prepare(.object(["action": .object(["op": .string(op), "args": args])])))
        }
        let title = "Bridge Test " + UUID().uuidString
        func cleanup() throws {
            let options = PHFetchOptions()
            options.predicate = NSPredicate(format: "title == %@", title)
            let albums = PHAssetCollection.fetchAssetCollections(with: .album, subtype: .any, options: options)
            guard albums.count <= 1 else { throw SourceFailure.corrupt }
            guard let album = albums.firstObject else { return }
            let assets = PHAsset.fetchAssets(in: album, options: nil)
            let assetIDs = (0..<assets.count).map { assets.object(at: $0).localIdentifier }
            let albumID = album.localIdentifier
            try PHPhotoLibrary.shared().performChangesAndWait {
                PHAssetChangeRequest.deleteAssets(assets)
                PHAssetCollectionChangeRequest.deleteAssetCollections([album] as NSArray)
            }
            guard PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [albumID], options: nil).count == 0,
                  PHAsset.fetchAssets(withLocalIdentifiers: assetIDs, options: nil).count == 0 else { throw SourceFailure.corrupt }
        }
        do {
            let created = try await command("photos.create_album", .object(["name": .string(title)]))
            let albumID = created["native_key"].string
            guard let album = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [albumID], options: nil).firstObject, album.localizedTitle == title else { throw SourceFailure.corrupt }
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
