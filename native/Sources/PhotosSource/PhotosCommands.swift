import Foundation
@preconcurrency import Photos
import CryptoKit
import SourceSupport

extension PhotosSource {
    public static let commands = ["photos.import", "photos.create_album", "photos.add_to_album", "photos.remove_from_album", "photos.set_favorite", "photos.delete"]
    public func prepare(_ command: JSON) async throws -> Prepared {
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        let action = command["action"]
        let op = action["op"].string
        let args = action["args"]
        guard Self.commands.contains(op), case .object = args else { throw SourceFailure.unsupported }
        let allowed: Set<String>
        switch op {
        case "photos.import": allowed = ["data", "digest", "kind", "album"]
        case "photos.create_album": allowed = ["name"]
        case "photos.set_favorite": allowed = ["asset", "favorite"]
        case "photos.delete": allowed = ["asset"]
        default: allowed = ["asset", "album"]
        }
        guard Set(args.object.keys).isSubset(of: allowed) else { throw SourceFailure.invalid }
        for (key, value) in args.object where key != "favorite" { guard case .string = value else { throw SourceFailure.invalid } }
        if op == "photos.import" {
            guard let bytes = Data(base64Encoded: args["data"].string), !bytes.isEmpty, bytes.count <= 16 << 20, ["image", "video"].contains(args["kind"].string), digest(bytes) == args["digest"].string else { throw SourceFailure.invalid }
        } else if op == "photos.create_album" {
            guard !args["name"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, args["name"].string.count <= 1024 else { throw SourceFailure.invalid }
        } else {
            guard !args["asset"].string.isEmpty else { throw SourceFailure.invalid }
            if op == "photos.set_favorite" { guard case .bool = args["favorite"] else { throw SourceFailure.invalid } }
            if op == "photos.delete", !importedAssets.contains(args["asset"].string) { throw SourceFailure.unsupported }
        }
        if ["photos.add_to_album", "photos.remove_from_album"].contains(op), args["album"].string.isEmpty { throw SourceFailure.invalid }
        let rows = try await snapshot()
        for (key, collection) in [("asset", "photos.asset"), ("album", "photos.album")] where !args[key].string.isEmpty {
            guard rows.contains(where: { $0["collection"].string == collection && $0["native_key"] == args[key] }) else { throw SourceFailure.unavailable }
        }
        return Prepared(nativeState: .array(rows.filter { $0["native_key"] == args["asset"] || $0["native_key"] == args["album"] }), fingerprint: command["request_hash"].string, action: action)
    }
    func digest(_ bytes: Data) -> String { SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined() }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let fresh = try await prepare(.object(["action": prepared.action]))
        guard fresh.nativeState == prepared.nativeState else { throw SourceFailure.conflict }
        let op = prepared.action["op"].string
        let args = prepared.action["args"]
        var key = args["asset"].string
        if fixture != nil {
            if op == "photos.create_album" {
                key = UUID().uuidString
                fixture!.append(row("photos.album", key, .object(["name": args["name"], "kind": .string("user"), "count": .number(0)])))
            } else if op == "photos.import" {
                key = UUID().uuidString
                let bytes = Data(base64Encoded: args["data"].string)!
                fixtureBlobs["photos.original/" + key] = bytes
                importedAssets.insert(key)
                fixture!.append(row("photos.asset", key, .object(["local_id": .string(key), "created": .string(timestamp()), "kind": args["kind"], "size": .null, "duration": .number(0), "favorite": .bool(false), "hidden": .bool(false), "location": .null, "albums": .array(args["album"].string.isEmpty ? [] : [args["album"]]), "cloud_state": .string("local")])))
            } else if op == "photos.delete" {
                fixture!.removeAll { $0["native_key"].string == key }
                importedAssets.remove(key)
                fixtureBlobs.removeValue(forKey: "photos.original/" + key)
            } else {
                guard let index = fixture!.firstIndex(where: { $0["native_key"].string == key }) else { throw SourceFailure.unavailable }
                var data = fixture![index]["data"].object
                if op == "photos.set_favorite" { data["favorite"] = args["favorite"] }
                else {
                    var albums = data["albums"]?.array ?? []
                    albums.removeAll { $0 == args["album"] }
                    if op == "photos.add_to_album" { albums.append(args["album"]) }
                    data["albums"] = .array(albums)
                }
                fixture![index] = row("photos.asset", key, .object(data))
            }
            for index in fixture!.indices where fixture![index]["collection"].string == "photos.album" {
                let album = fixture![index]["native_key"]
                var data = fixture![index]["data"].object
                data["count"] = .number(Double(fixture!.filter { $0["collection"].string == "photos.asset" && $0["data"]["albums"].array.contains(album) }.count))
                fixture![index] = row("photos.album", album.string, .object(data))
            }
        } else {
            key = try changeLibrary(op, args)
        }
        let rows = try await snapshot()
        let observed = rows.first { $0["native_key"].string == key }
        guard op == "photos.delete" ? observed == nil : observed != nil else { throw SourceFailure.unavailable }
        if op == "photos.create_album", observed?["data"]["name"] != args["name"] { throw SourceFailure.unavailable }
        if op == "photos.import", !args["album"].string.isEmpty, observed?["data"]["albums"].array.contains(args["album"]) != true { throw SourceFailure.unavailable }
        if op == "photos.import" {
            let original = try await blob("photos.original", .object(["asset": .string(key)]))
            guard original["digest"] == args["digest"] else { throw SourceFailure.unavailable }
        }
        if op == "photos.set_favorite", observed?["data"]["favorite"] != args["favorite"] { throw SourceFailure.unavailable }
        if ["photos.add_to_album", "photos.remove_from_album"].contains(op), observed?["data"]["albums"].array.contains(args["album"]) != (op == "photos.add_to_album") { throw SourceFailure.unavailable }
        return .object(["native_key": .string(key), "fingerprint": .string(prepared.fingerprint), "observed": .bool(true), "absent": .bool(op == "photos.delete"), "digest": args["digest"]])
    }
    public func captureCommand(_ evidence: JSON, into sink: any ObservationSink) async throws {
        let started = timestamp()
        var rows = try await snapshot()
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        if evidence["absent"].bool {
            guard !rows.contains(where: { $0["native_key"] == evidence["native_key"] }) else { throw SourceFailure.conflict }
            rows.append(.object(["kind": .string("absent"), "collection": .string("photos.asset"), "native_key": evidence["native_key"], "scoped_alias": .string("photos/photos.asset/" + evidence["native_key"].string), "revalidated": .bool(true)]))
        }
        let stamped = rows.map { row in var fields = row.object; fields["observed_at"] = .string(started); return JSON.object(fields) }
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: stamped, scanID: UUID().uuidString, scanStartedAt: started, scope: [], completionEvidence: "", blobHandles: [:], live: false)
    }
    func changeLibrary(_ op: String, _ args: JSON) throws -> String {
        let asset = PHAsset.fetchAssets(withLocalIdentifiers: [args["asset"].string], options: nil).firstObject
        let album = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [args["album"].string], options: nil).firstObject
        if let album, !album.canPerform(.addContent) { throw SourceFailure.unsupported }
        var key = args["asset"].string
        var temporary: URL?
        if op == "photos.import", args["kind"].string == "video" {
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".mov")
            try Data(base64Encoded: args["data"].string)!.write(to: url, options: .atomic)
            temporary = url
        }
        defer { if let temporary { try? FileManager.default.removeItem(at: temporary) } }
        try PHPhotoLibrary.shared().performChangesAndWait {
            switch op {
            case "photos.create_album": key = PHAssetCollectionChangeRequest.creationRequestForAssetCollection(withTitle: args["name"].string).placeholderForCreatedAssetCollection.localIdentifier
            case "photos.import":
                let request = PHAssetCreationRequest.forAsset()
                if let temporary { request.addResource(with: .video, fileURL: temporary, options: nil) }
                else { request.addResource(with: .photo, data: Data(base64Encoded: args["data"].string)!, options: nil) }
                guard let placeholder = request.placeholderForCreatedAsset else { return }
                key = placeholder.localIdentifier
                if let album { PHAssetCollectionChangeRequest(for: album)?.addAssets([placeholder] as NSArray) }
            case "photos.delete": if let asset { PHAssetChangeRequest.deleteAssets([asset] as NSArray) }
            case "photos.set_favorite": if let asset { PHAssetChangeRequest(for: asset).isFavorite = args["favorite"].bool }
            default:
                if let album, let asset, let request = PHAssetCollectionChangeRequest(for: album) {
                    if op == "photos.add_to_album" { request.addAssets([asset] as NSArray) }
                    else { request.removeAssets([asset] as NSArray) }
                }
            }
        }
        if op == "photos.import" { importedAssets.insert(key) }
        if op == "photos.delete" { importedAssets.remove(key) }
        return key
    }
}
