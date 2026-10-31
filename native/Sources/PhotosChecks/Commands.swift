import Foundation
import AppKit
import CryptoKit
import PhotosSource
import SourceSupport

func commandChecks() async throws {
    let source = PhotosSource(fixture: [])
    func run(_ op: String, _ args: JSON) async throws -> JSON {
        let prepared = try await source.prepare(.object(["action": .object(["op": .string("photos." + op), "args": args])]))
        return try await source.dispatch(prepared)
    }
    let album = try await run("create_album", .object(["name": .string("[ab-test] Photos commands")]))["native_key"]
    for kind in ["image", "video"] {
        var bytes = Data(("synthetic " + kind).utf8)
        if kind == "image" {
            guard let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.corrupt }
            for x in 0..<2 { for y in 0..<2 { image.setColor(.red, atX: x, y: y) } }
            guard let png = image.representation(using: .png, properties: [:]) else { throw SourceFailure.corrupt }
            bytes = png
        }
        await source.delayNextFixtureObservationForProof(2)
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        let imported = try await run("import", .object(["data": .string(bytes.base64EncodedString()), "digest": .string(digest), "kind": .string(kind), "album": album]))
        let asset = imported["native_key"]
        let blob = try await source.blob("photos.original", .object(["asset": asset]))
        guard blob["digest"].string == digest, blob["size"].number == Double(bytes.count), await source.importedAssetIdentifiersForProof() == [asset.string] else { throw SourceFailure.corrupt }
        let original = try await source.readBlob(blob["handle"].string, offset: 0, length: 131072)
        guard original["data"].string == bytes.base64EncodedString(), original["eof"].bool else { throw SourceFailure.corrupt }
        await source.delayNextFixtureObservationForProof(2)
        _ = try await run("set_favorite", .object(["asset": asset, "favorite": .bool(true)]))
        guard try await source.search(.object(["favorite": .bool(true), "album": album]))["items"].array.count == 1 else { throw SourceFailure.corrupt }
        let rows = try await source.snapshot()
        guard rows.first(where: { $0["native_key"] == album })?["data"]["count"].number == 1 else { throw SourceFailure.corrupt }
        _ = try await run("set_favorite", .object(["asset": asset, "favorite": .bool(false)]))
        guard try await source.search(.object(["favorite": .bool(true)]))["items"].array.isEmpty else { throw SourceFailure.corrupt }
        _ = try await run("remove_from_album", .object(["asset": asset, "album": album]))
        guard try await source.search(.object(["album": album]))["items"].array.isEmpty else { throw SourceFailure.corrupt }
        _ = try await run("add_to_album", .object(["asset": asset, "album": album]))
        guard try await source.search(.object(["album": album]))["items"].array.count == 1 else { throw SourceFailure.corrupt }
        await source.delayNextFixtureObservationForProof(2)
        _ = try await run("delete", .object(["asset": asset]))
        guard try await source.search(.object([:]))["items"].array.isEmpty, await source.importedAssetIdentifiersForProof().isEmpty else { throw SourceFailure.corrupt }
    }
    let external: JSON = .object(["kind": .string("upsert"), "collection": .string("photos.asset"), "native_key": .string("external"), "data": .object([:])])
    try await source.replaceFixture([external])
    do { _ = try await run("delete", .object(["asset": .string("external")])); throw SourceFailure.corrupt } catch SourceFailure.unsupported {}
    do { _ = try await run("import", .object(["data": .string("YQ=="), "digest": .string("bad"), "kind": .string("image")])); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
    print("commands_readback=passed")
}
