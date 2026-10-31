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
        for bundle in ["com.apple.systemevents"] {
            let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else {
                return .object(["grant_passed": .bool(true), "failure": .string("Photos cleanup needs Helper Automation access to System Events. Enable it in System Settings before retrying.")])
            }
        }
        func script(_ body: String) throws -> NSAppleEventDescriptor {
            guard let script = NSAppleScript(source: "with timeout of 15 seconds\n" + body + "\nend timeout") else { throw SourceFailure.invalid }
            var error: NSDictionary?
            let result = script.executeAndReturnError(&error)
            guard error == nil else { throw SourceFailure.unavailable }
            return result
        }
        let filename = title + ".png"
        await source.setImportFilenameForProof(filename)
        func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
            var value: CFTypeRef?
            guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else { return nil }
            return value
        }
        func displaysFilename(_ root: AXUIElement, excluding mainWindow: AXUIElement, filename: String) -> Bool {
            let windows = attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? []
            let panels = windows.filter { !CFEqual($0, mainWindow) }
            guard windows.count == 2, panels.count == 1 else { return false }
            var budget = 4000
            func matches(_ node: AXUIElement, _ depth: Int) -> Bool {
                budget -= 1
                guard budget > 0, depth < 25 else { return false }
                if [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains(where: { (attribute(node, $0) as? String ?? "").contains(filename) }) { return true }
                return (attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? []).contains { matches($0, depth + 1) }
            }
            return matches(panels[0], 0)
        }
        var stage = "library read"
        func deleteVerifiedAsset(_ assetID: String, filename: String) async throws {
            guard PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count != 0 else { return }
            stage = "opening the imported test photo"
            var link = URLComponents()
            link.scheme = "photos"
            link.host = "asset"
            link.queryItems = [URLQueryItem(name: "identifier", value: assetID)]
            guard let url = link.url, NSWorkspace.shared.open(url) else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .seconds(1))
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Photos").first,
                  app.activate() else { throw SourceFailure.unavailable }
            let root = AXUIElementCreateApplication(app.processIdentifier)
            let windows = attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? []
            guard windows.count == 1 else { throw SourceFailure.conflict }
            let mainWindow = windows[0]
            stage = "identifying the imported test photo"
            _ = try script("""
            tell application "System Events" to tell process "Photos"
                if frontmost is false then error "Photos must be in front for cleanup."
                if (count windows) is not 1 then error "Close other Photos windows before retrying."
                if (count sheets of window 1) is not 0 then error "Close the Photos dialog before retrying."
                keystroke "i" using command down
            end tell
            """)
            var infoOpen = true
            defer {
                if infoOpen {
                    _ = try? script("tell application \"System Events\" to tell process \"Photos\"\nif frontmost then keystroke \"i\" using command down\nend tell")
                }
            }
            var identified = false
            for _ in 0..<40 {
                identified = displaysFilename(root, excluding: mainWindow, filename: filename)
                if identified { break }
                try await Task.sleep(for: .milliseconds(150))
            }
            guard identified else { throw SourceFailure.conflict }
            _ = try script("""
            tell application "System Events" to tell process "Photos"
                if frontmost is false then error "Photos must be in front for cleanup."
                keystroke "i" using command down
            end tell
            """)
            infoOpen = false
            try await Task.sleep(for: .milliseconds(200))
            _ = try script("""
            tell application "System Events" to tell process "Photos"
                if frontmost is false then error "Photos must be in front for cleanup."
                if (count windows) is not 1 then error "Close other Photos windows before retrying."
                if (count sheets of window 1) is not 0 then error "Close the Photos dialog before retrying."
                key code 51 using {command down, option down}
            end tell
            """)
            stage = "deleting the imported test photo"
            for _ in 0..<80 {
                if PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count == 0 { break }
                try await Task.sleep(for: .milliseconds(150))
            }
            guard PHAsset.fetchAssets(withLocalIdentifiers: [assetID], options: nil).count == 0 else { throw SourceFailure.unavailable }
        }
        func cleanup() async throws {
            let owned = await source.importedAssetIdentifiersForProof()
            guard owned.count <= 1 else { throw SourceFailure.conflict }
            for assetID in owned { try await deleteVerifiedAsset(assetID, filename: filename) }
        }
        do {
            var recovered = 0
            if let recoveryPath, let recoveryDigest {
                stage = "previous test photo cleanup"
                let data = try Data(contentsOf: URL(fileURLWithPath: recoveryPath))
                guard SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == recoveryDigest else { throw SourceFailure.invalid }
                let manifest = try JSONDecoder().decode(JSON.self, from: data)
                guard manifest["owner"].string.hasPrefix("[ab-test] F3 "), manifest["assets"].array.count <= 3 else { throw SourceFailure.invalid }
                guard let legacy = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
                for x in 0..<2 { for y in 0..<2 { legacy.setColor(.red, atX: x, y: y) } }
                guard let legacyBytes = legacy.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
                let legacyDigest = SHA256.hash(data: legacyBytes).map { String(format: "%02x", $0) }.joined()
                let all = PHAsset.fetchAssets(with: nil)
                var verified: [(String, String)] = []
                for item in manifest["assets"].array {
                    guard UUID(uuidString: item["uuid"].string) != nil, item["creator"].string == "com.johansardar.applebridge.services" else { throw SourceFailure.invalid }
                    var candidates: [PHAsset] = []
                    all.enumerateObjects { asset, _, _ in
                        if asset.localIdentifier.split(separator: "/").first.map(String.init) == item["uuid"].string { candidates.append(asset) }
                    }
                    guard candidates.count <= 1 else { throw SourceFailure.conflict }
                    guard let asset = candidates.first else { continue }
                    guard asset.pixelWidth == 2, asset.pixelHeight == 2,
                          let created = asset.creationDate, abs(created.timeIntervalSinceReferenceDate - item["created"].number) < 1,
                          let resource = PHAssetResource.assetResources(for: asset).first(where: { $0.type == .photo }), resource.originalFilename == item["filename"].string else { throw SourceFailure.conflict }
                    let blob = try await source.blob("photos.original", .object(["asset": .string(asset.localIdentifier)]))
                    guard blob["digest"].string == legacyDigest, blob["size"].number == Double(legacyBytes.count) else { throw SourceFailure.conflict }
                    verified.append((asset.localIdentifier, resource.originalFilename))
                }
                for (assetID, expectedFilename) in verified {
                    try await deleteVerifiedAsset(assetID, filename: expectedFilename)
                    recovered += 1
                }
            } else if recoveryPath != nil || recoveryDigest != nil { throw SourceFailure.invalid }
            let options = PHFetchOptions()
            options.includeHiddenAssets = true
            let libraryCount = PHAsset.fetchAssets(with: options).count
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
            stage = "test photo import"
            let imported = try await command("photos.import", .object(["data": .string(bytes.base64EncodedString()), "digest": .string(digest), "kind": .string("image")]))
            let assetID = imported["native_key"].string
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
            result["cleanup_passed"] = .bool(true)
            return .object(result)
        } catch {
            let failedStage = stage
            let cleaned = (try? await cleanup()) != nil
            return .object(["failure": .string("Photos \(failedStage) failed. Check Helper Photos access and close any Photos dialog before retrying."), "grant_passed": .bool(true), "cleanup_passed": .bool(cleaned), "library_asset_count": .number(Double(PHAsset.fetchAssets(with: nil).count))])
        }
    }
}
#endif
