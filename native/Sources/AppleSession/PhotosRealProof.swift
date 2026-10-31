#if LAB
import Foundation
import AppKit
import Darwin
import Carbon
import ApplicationServices
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
        func command(_ op: String, _ args: JSON) async throws -> JSON {
            try await source.dispatch(source.prepare(.object(["action": .object(["op": .string(op), "args": args])])))
        }
        let title = "[ab-test] " + UUID().uuidString
        guard AXIsProcessTrusted() else {
            return .object(["grant_passed": .bool(true), "failure": .string("Photos cleanup needs Helper Accessibility access. Enable it in System Settings before retrying.")])
        }
        for bundle in ["com.apple.Photos", "com.apple.systemevents"] {
            let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else {
                return .object(["grant_passed": .bool(true), "failure": .string("Photos cleanup needs Helper Automation access to Photos and System Events. Enable it in System Settings before retrying.")])
            }
        }
        func script(_ body: String) throws -> NSAppleEventDescriptor {
            guard let script = NSAppleScript(source: "with timeout of 15 seconds\n" + body + "\nend timeout") else { throw SourceFailure.invalid }
            var error: NSDictionary?
            let result = script.executeAndReturnError(&error)
            guard error == nil else { throw SourceFailure.unavailable }
            return result
        }
        func quoted(_ value: String) -> String {
            "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
        }
        func cleanup() async throws {
            let options = PHFetchOptions()
            options.predicate = NSPredicate(format: "title == %@", title)
            let albums = PHAssetCollection.fetchAssetCollections(with: .album, subtype: .any, options: options)
            guard albums.count <= 1 else { throw SourceFailure.corrupt }
            guard let album = albums.firstObject else { return }
            let owned = await source.importedAssetIdentifiersForProof()
            let assets = PHAsset.fetchAssets(in: album, options: nil)
            let assetIDs = (0..<assets.count).map { assets.object(at: $0).localIdentifier }
            guard assetIDs.count <= 1, Set(assetIDs).isSubset(of: owned) else { throw SourceFailure.conflict }
            for assetID in owned {
                guard PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count != 0 else { continue }
                let identity = quoted(assetID)
                _ = try script("""
                tell application "Photos"
                    activate
                    set targetPhoto to media item id \(identity)
                    spotlight targetPhoto
                end tell
                """)
                var selected = false
                for _ in 0..<40 {
                    selected = try script("""
                    tell application "Photos"
                        set chosen to selection
                        if (count chosen) is not 1 then return false
                        return (id of item 1 of chosen) is \(identity)
                    end tell
                    """).booleanValue
                    if selected { break }
                    try await Task.sleep(for: .milliseconds(150))
                }
                guard selected else { throw SourceFailure.conflict }
                _ = try script("""
                tell application "Photos"
                    set chosen to selection
                    if (count chosen) is not 1 then error "Photos cleanup selection changed."
                    if (id of item 1 of chosen) is not \(identity) then error "Photos cleanup selection changed."
                end tell
                tell application "System Events" to tell process "Photos"
                    if frontmost is false then error "Photos must be in front for cleanup."
                    if (count windows) is not 1 then error "Close other Photos windows before retrying."
                    if (count sheets of window 1) is not 0 then error "Close the Photos dialog before retrying."
                    key code 51 using {command down, option down}
                end tell
                """)
                for _ in 0..<80 {
                    if PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count == 0 { break }
                    try await Task.sleep(for: .milliseconds(150))
                }
                guard PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count == 0 else { throw SourceFailure.unavailable }
            }
            _ = try script("""
            tell application "Photos"
                set targetAlbum to album id \(quoted(album.localIdentifier))
                if name of targetAlbum is not \(quoted(title)) then error "Photos cleanup album changed."
                if (count media items of targetAlbum) is not 0 then error "Photos cleanup album is not empty."
                delete targetAlbum
            end tell
            """)
            for _ in 0..<40 {
                if PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [album.localIdentifier], options: nil).count == 0 { return }
                try await Task.sleep(for: .milliseconds(150))
            }
            throw SourceFailure.unavailable
        }
        var stage = "library read"
        do {
            let options = PHFetchOptions()
            options.includeHiddenAssets = true
            let libraryCount = PHAsset.fetchAssets(with: options).count
            let snapshot = try await source.snapshot()
            guard snapshot.filter({ $0["collection"].string == "photos.asset" }).count == libraryCount else { throw SourceFailure.corrupt }
            stage = "test album creation"
            let created = try await command("photos.create_album", .object(["name": .string(title)]))
            let albumID = created["native_key"].string
            guard let album = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [albumID], options: nil).firstObject, album.localizedTitle == title else { throw SourceFailure.corrupt }
            guard let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
            for x in 0..<2 { for y in 0..<2 { image.setColor(.red, atX: x, y: y) } }
            guard let bytes = image.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
            let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
            stage = "test photo import"
            let imported = try await command("photos.import", .object(["data": .string(bytes.base64EncodedString()), "digest": .string(digest), "kind": .string("image"), "album": .string(albumID)]))
            let assetID = imported["native_key"].string
            let rows = try await source.search(.object(["album": .string(albumID)]))["items"].array
            guard rows.count == 1, rows[0]["local_id"].string == assetID else { throw SourceFailure.corrupt }
            stage = "original byte verification"
            let original = try await source.blob("photos.original", .object(["asset": .string(assetID)]))
            guard original["digest"].string == digest, original["size"].number == Double(bytes.count) else { throw SourceFailure.corrupt }
            let read = try await source.readBlob(original["handle"].string, offset: 0, length: 131072)
            guard read["data"].string == bytes.base64EncodedString(), read["eof"].bool else { throw SourceFailure.corrupt }
            var result: [String: JSON] = ["grant_passed": .bool(true), "read_passed": .bool(true), "import_passed": .bool(true), "original_passed": .bool(true), "asset_count": .number(1), "library_asset_count": .number(Double(libraryCount)), "original_digest": .string(digest)]
            stage = "test photo cleanup"
            try await cleanup()
            result["cleanup_passed"] = .bool(true)
            return .object(result)
        } catch {
            let cleaned = (try? await cleanup()) != nil
            return .object(["failure": .string("Photos \(stage) failed. Check Helper Photos access and close any Photos dialog before retrying."), "grant_passed": .bool(true), "cleanup_passed": .bool(cleaned), "library_asset_count": .number(Double(PHAsset.fetchAssets(with: nil).count))])
        }
    }
}
#endif
