#if LAB
import Foundation
import AppKit
import ApplicationServices
import Carbon
@preconcurrency import Photos
import CryptoKit
import PhotosSource
import SourceSupport

@MainActor
enum PhotosRealProof {
    static func run(_ scenario: String, recoveryPath: String? = nil, recoveryDigest: String? = nil) async throws -> JSON {
        guard ["PHO-READ-REAL", "PHO-IMPORT-REAL", "PHO-ORIGINAL-REAL", "PHO-IDLE"].contains(scenario) else { throw SourceFailure.invalid }
        guard PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized else {
            return .object(["grant_passed": .bool(false)])
        }
        let source = PhotosSource()
        func command(_ op: String, _ args: JSON) async throws -> JSON {
            try await source.dispatch(source.prepare(.object(["action": .object(["op": .string(op), "args": args])])))
        }
        let title = "[ab-test] " + UUID().uuidString
        guard AXIsProcessTrusted() else {
            return .object(["grant_passed": .bool(true), "failure": .string("Photos cleanup needs Helper Accessibility access. Enable it in System Settings before retrying.")])
        }
        let filename = title + ".png"
        var stage = "library read"
        var cleanupDiagnostics: [String: JSON] = [:]
        let ownedURL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/photos-real-owned.json")
        var recoveryComplete = recoveryPath == nil && recoveryDigest == nil && !FileManager.default.fileExists(atPath: ownedURL.path)
        var importedDigest = ""
        var importedSize = 0
        var baselineCount = 0
        var currentCleanupComplete = false
        var importStarted = Date()
        func writeOwned(_ assetID: String = "") throws {
            let entry: JSON = .object(["baseline_count": .number(Double(baselineCount)), "owner": .string("[ab-test] F3"), "asset": .string(assetID), "filename": .string(filename), "digest": .string(importedDigest), "size": .number(Double(importedSize)), "created": .number(importStarted.timeIntervalSinceReferenceDate)])
            let data = try JSONEncoder().encode(entry)
            try data.write(to: ownedURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: ownedURL.path)
        }
        let ui = try PhotosProofUI()
        func deleteVerifiedAsset(_ assetID: String, filename: String, created: Date) async throws {
            stage = "cleaning up the verified test photo"
            do {
                if assetID.isEmpty { try await ui.purge([PhotosProofAsset(id: "", filename: filename, created: created)]) }
                else { try await ui.remove(assetID, filename: filename, created: created) }
            }
            catch {
                stage = ui.stage
                cleanupDiagnostics = ui.diagnostics
                throw error
            }
        }
        func cleanup() async throws {
            guard !currentCleanupComplete else { return }
            let owned = await source.importedAssetIdentifiersForProof()
            guard owned.count <= 1 else { throw SourceFailure.conflict }
            for assetID in owned {
                if let asset = PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).firstObject {
                    guard asset.pixelWidth == 2, asset.pixelHeight == 2,
                          let created = asset.creationDate, abs(created.timeIntervalSince(importStarted)) < 1,
                          PHAssetResource.assetResources(for: asset).contains(where: { $0.type == .photo && $0.originalFilename == filename }) else { throw SourceFailure.conflict }
                    let blob = try await source.blob("photos.original", .object(["asset": .string(assetID)]))
                    guard blob["digest"].string == importedDigest, blob["size"].number == Double(importedSize) else { throw SourceFailure.conflict }
                }
                try await deleteVerifiedAsset(assetID, filename: filename, created: importStarted)
            }
            guard !owned.isEmpty else { return }
            let options = PHFetchOptions()
            options.includeHiddenAssets = true
            guard PHAsset.fetchAssets(with: options).count == baselineCount else { throw SourceFailure.conflict }
            if FileManager.default.fileExists(atPath: ownedURL.path), recoveryComplete { try FileManager.default.removeItem(at: ownedURL) }
            currentCleanupComplete = true
        }
        do {
            var recovered = 0
            if FileManager.default.fileExists(atPath: ownedURL.path) {
                stage = "previous tagged test photo verification"
                let entry = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: ownedURL))
                let name = entry["filename"].string
                guard entry["owner"].string == "[ab-test] F3", name.hasPrefix("[ab-test] "), name.hasSuffix(".png"),
                      UUID(uuidString: String(name.dropFirst(10).dropLast(4))) != nil else { throw SourceFailure.invalid }
                let all = PHAsset.fetchAssets(with: nil)
                var matches: [PHAsset] = []
                all.enumerateObjects { asset, _, _ in
                    if PHAssetResource.assetResources(for: asset).contains(where: { $0.type == .photo && $0.originalFilename == name }) { matches.append(asset) }
                }
                guard matches.count <= 1 else { throw SourceFailure.conflict }
                for asset in matches {
                    guard entry["asset"].string.isEmpty || asset.localIdentifier == entry["asset"].string,
                          asset.pixelWidth == 2, asset.pixelHeight == 2,
                          let created = asset.creationDate, abs(created.timeIntervalSinceReferenceDate - entry["created"].number) < 1 else { throw SourceFailure.conflict }
                    let blob = try await source.blob("photos.original", .object(["asset": .string(asset.localIdentifier)]))
                    guard blob["digest"] == entry["digest"], blob["size"] == entry["size"] else { throw SourceFailure.conflict }
                    try await deleteVerifiedAsset(asset.localIdentifier, filename: name, created: created)
                    recovered += 1
                }
                if matches.isEmpty {
                    try await deleteVerifiedAsset(entry["asset"].string, filename: name, created: Date(timeIntervalSinceReferenceDate: entry["created"].number))
                }
                let options = PHFetchOptions()
                options.includeHiddenAssets = true
                guard entry["baseline_count"].number == Double(PHAsset.fetchAssets(with: options).count) else { throw SourceFailure.conflict }
                try FileManager.default.removeItem(at: ownedURL)
                if recoveryPath == nil && recoveryDigest == nil { recoveryComplete = true }
            }
            if let recoveryPath, let recoveryDigest {
                stage = "previous test photo cleanup"
                let data = try Data(contentsOf: URL(fileURLWithPath: recoveryPath))
                guard SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == recoveryDigest else { throw SourceFailure.invalid }
                let manifest = try JSONDecoder().decode(JSON.self, from: data)
                guard manifest["owner"].string.hasPrefix("[ab-test] F3 "), manifest["assets"].array.count == 3, Set(manifest["assets"].array.map { $0["uuid"].string }).count == 3 else { throw SourceFailure.invalid }
                guard let legacy = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
                for x in 0..<2 { for y in 0..<2 { legacy.setColor(.red, atX: x, y: y) } }
                guard let legacyBytes = legacy.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
                let legacyDigest = SHA256.hash(data: legacyBytes).map { String(format: "%02x", $0) }.joined()
                let all = PHAsset.fetchAssets(with: nil)
                var liveCount = 0
                var verified: [(String, String, Date)] = []
                for item in manifest["assets"].array {
                    guard UUID(uuidString: item["uuid"].string) != nil, item["creator"].string == "com.johansardar.applebridge.services" else { throw SourceFailure.invalid }
                    var candidates: [PHAsset] = []
                    all.enumerateObjects { asset, _, _ in
                        if asset.localIdentifier.split(separator: "/").first.map(String.init) == item["uuid"].string { candidates.append(asset) }
                    }
                    guard candidates.count <= 1 else { throw SourceFailure.conflict }
                    guard let asset = candidates.first else {
                        verified.append((item["uuid"].string, item["filename"].string, Date(timeIntervalSinceReferenceDate: item["created"].number)))
                        continue
                    }
                    liveCount += 1
                    guard asset.pixelWidth == 2, asset.pixelHeight == 2,
                          let created = asset.creationDate, abs(created.timeIntervalSinceReferenceDate - item["created"].number) < 1,
                          let resource = PHAssetResource.assetResources(for: asset).first(where: { $0.type == .photo }), resource.originalFilename == item["filename"].string else { throw SourceFailure.conflict }
                    let blob = try await source.blob("photos.original", .object(["asset": .string(asset.localIdentifier)]))
                    guard blob["digest"].string == legacyDigest, blob["size"].number == Double(legacyBytes.count) else { throw SourceFailure.conflict }
                    verified.append((asset.localIdentifier, resource.originalFilename, created))
                }
                let expectedCount = all.count - liveCount
                let targets = verified.map { PhotosProofAsset(id: $0.0, filename: $0.1, created: $0.2) }
                do {
                    try await ui.trash(targets)
                    guard PHAsset.fetchAssets(with: nil).count == expectedCount else { throw SourceFailure.conflict }
                    cleanupDiagnostics["recovered_library_asset_count"] = .number(Double(expectedCount))
                    try await ui.purge(targets)
                } catch {
                    stage = ui.stage
                    cleanupDiagnostics.merge(ui.diagnostics) { _, new in new }
                    throw error
                }
                recovered += verified.count
                recoveryComplete = true
            } else if recoveryPath != nil || recoveryDigest != nil { throw SourceFailure.invalid }
            let options = PHFetchOptions()
            options.includeHiddenAssets = true
            let libraryCount = PHAsset.fetchAssets(with: options).count
            baselineCount = libraryCount
            do { try await ui.openTrash() }
            catch {
                stage = ui.stage
                cleanupDiagnostics.merge(ui.diagnostics) { _, new in new }
                throw error
            }
            let snapshot = try await source.snapshot()
            guard snapshot.filter({ $0["collection"].string == "photos.asset" }).count == libraryCount else { throw SourceFailure.corrupt }
            guard let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
            let colors = Array(SHA256.hash(data: Data(title.utf8)))
            for x in 0..<2 { for y in 0..<2 {
                let offset = (x * 2 + y) * 3
                image.setColor(NSColor(deviceRed: CGFloat(colors[offset]) / 255, green: CGFloat(colors[offset + 1]) / 255, blue: CGFloat(colors[offset + 2]) / 255, alpha: 1), atX: x, y: y)
            } }
            guard let bytes = image.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
            let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
            importedDigest = digest
            importedSize = bytes.count
            importStarted = Date()
            await source.setImportFilenameForProof(filename, created: importStarted)
            try writeOwned()
            stage = "test photo import"
            let imported = try await command("photos.import", .object(["data": .string(bytes.base64EncodedString()), "digest": .string(digest), "kind": .string("image")]))
            let assetID = imported["native_key"].string
            try writeOwned(assetID)
            let rows = try await source.snapshot().filter { $0["collection"].string == "photos.asset" && $0["native_key"].string == assetID }
            guard rows.count == 1, rows[0]["data"]["local_id"].string == assetID else { throw SourceFailure.corrupt }
            stage = "original byte verification"
            let original = try await source.blob("photos.original", .object(["asset": .string(assetID)]))
            guard original["digest"].string == digest, original["size"].number == Double(bytes.count) else { throw SourceFailure.corrupt }
            let read = try await source.readBlob(original["handle"].string, offset: 0, length: 131072)
            guard read["data"].string == bytes.base64EncodedString(), read["eof"].bool else { throw SourceFailure.corrupt }
            var result: [String: JSON] = ["grant_passed": .bool(true), "read_passed": .bool(true), "import_passed": .bool(true), "original_passed": .bool(true), "asset_count": .number(1), "recovered_test_asset_count": .number(Double(recovered)), "library_asset_count": .number(Double(libraryCount)), "original_digest": .string(digest)]
            stage = "test photo cleanup"
            try await cleanup()
            let remaining = PHAsset.fetchAssets(with: options).count
            guard remaining == libraryCount else { throw SourceFailure.conflict }
            result["cleanup_library_asset_count"] = .number(Double(remaining))
            result["cleanup_passed"] = .bool(true)
            return .object(result)
        } catch {
            let failedStage = stage
            ui.inspect()
            cleanupDiagnostics.merge(ui.diagnostics) { _, new in new }
            let currentCleaned = (try? await cleanup()) != nil
            let cleaned = currentCleaned && recoveryComplete
            let failure = ui.diagnostics["recently_deleted_locked_passed"] == .bool(true)
                ? "Photos Recently Deleted is locked. On the Air, open Recently Deleted in Photos, authenticate to unlock the album, and leave Photos open before retrying."
                : "Photos \(failedStage) failed. Close any Photos dialog and retry; the private test-photo recovery record has been retained."
            return .object(["failure": .string(failure), "grant_passed": .bool(true), "cleanup_passed": .bool(cleaned), "cleanup_diagnostics": .object(cleanupDiagnostics), "library_asset_count": .number(Double(PHAsset.fetchAssets(with: nil).count))])
        }
    }
}
#endif
