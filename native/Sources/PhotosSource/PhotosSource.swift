import Foundation
@preconcurrency import Photos
import CryptoKit
import SourceSupport

public actor PhotosSource: Source {
    public nonisolated let name = "photos"
    var fixture: [JSON]?
    let generation = UUID().uuidString
    var fixtureBlobs: [String: Data]
    private var blobs: [String: (Data, Date, String, String)] = [:]
    private var activeBlobRequests = 0
    public init(fixture: [JSON]? = nil, fixtureBlobs: [String: Data] = [:]) { self.fixture = fixture; self.fixtureBlobs = fixtureBlobs }
    public func replaceFixture(_ rows: [JSON]) throws {
        guard fixture != nil else { throw SourceFailure.unsupported }
        fixture = rows
    }
    public static let coverage: JSON = .object(["fields": .string("partial"), "ocr_index_enabled": .bool(false), "missing": .array([.string("person and keyword unavailable through public PhotoKit"), .string("byte size and cloud download state unavailable through public PhotoKit")])])
    public func open() async -> SourceReadiness {
        let status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        if fixture != nil || status == .authorized || status == .limited {
            return SourceReadiness(state: "ready", coverage: Self.coverage, capabilities: ["photos.search", "photos.original", "photos.thumbnail"] + Self.commands)
        }
        return SourceReadiness(state: "permission_missing", coverage: Self.coverage, reason: status == .notDetermined ? "Photos readWrite authorization is not_determined; prompt forbidden" : "Photos readWrite authorization denied or restricted")
    }
    public func snapshot() async throws -> [JSON] {
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        if let fixture { return fixture }
        var rows: [JSON] = []
        var memberships: [String: [JSON]] = [:]
        var membershipCount = 0
        for type in [PHAssetCollectionType.album, .smartAlbum] {
            let albums = PHAssetCollection.fetchAssetCollections(with: type, subtype: .any, options: nil)
            for index in 0..<albums.count {
                try Task.checkCancellation()
                let album = albums.object(at: index)
                let assets = PHAsset.fetchAssets(in: album, options: nil)
                membershipCount += assets.count
                guard albums.count <= 10000, assets.count <= 10000, membershipCount <= 100000 else { throw SourceFailure.unavailable }
                for i in 0..<assets.count { memberships[assets.object(at: i).localIdentifier, default: []].append(.string(album.localIdentifier)) }
                let kind = type == .smartAlbum ? "smart" : album.assetCollectionSubtype == .albumCloudShared ? "shared" : "user"
                rows.append(row("photos.album", album.localIdentifier, .object(["name": .string(album.localizedTitle ?? ""), "kind": .string(kind), "count": .number(Double(assets.count))])))
            }
        }
        let options = PHFetchOptions()
        options.includeHiddenAssets = true
        options.fetchLimit = 10001
        let assets = PHAsset.fetchAssets(with: options)
        guard assets.count <= 10000 else { throw SourceFailure.unavailable }
        for index in 0..<assets.count {
            try Task.checkCancellation()
            let asset = assets.object(at: index)
            let kind = asset.mediaType == .video ? "video" : asset.mediaSubtypes.contains(.photoLive) ? "live" : asset.mediaSubtypes.contains(.photoScreenshot) ? "screenshot" : "image"
            var location: JSON = .null
            if let value = asset.location { location = .object(["latitude": .number(value.coordinate.latitude), "longitude": .number(value.coordinate.longitude)]) }
            rows.append(row("photos.asset", asset.localIdentifier, .object(["local_id": .string(asset.localIdentifier), "created": asset.creationDate.map { .string(timestamp($0)) } ?? .null, "kind": .string(kind), "size": .null, "duration": .number(asset.duration), "favorite": .bool(asset.isFavorite), "hidden": .bool(asset.isHidden), "location": location, "albums": .array(memberships[asset.localIdentifier] ?? []), "cloud_state": .string("unknown") ])))
        }
        return rows
    }
    func row(_ collection: String, _ key: String, _ data: JSON) -> JSON {
        .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string(name + "/" + collection + "/" + key), "data": data])
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        var previous: [JSON]?
        while !Task.isCancelled {
            let started = timestamp()
            let rows = try await snapshot()
            guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
            let complete = fixture != nil || PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized
            if rows != previous {
                let stamped = rows.map { value in
                    var object = value.object
                    object["observed_at"] = .string(started)
                    return JSON.object(object)
                }
                try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: stamped, scanID: UUID().uuidString, scanStartedAt: started, scope: complete ? ["photos.asset", "photos.album"] : [], completionEvidence: complete ? "complete" : "", blobHandles: [:], live: false)
                previous = rows
            }
            try await Task.sleep(for: .seconds(30))
        }
    }
    public func search(_ args: JSON) async throws -> JSON {
        guard case .object = args else { throw SourceFailure.invalid }
        let allowed: Set<String> = ["from", "to", "album", "kind", "favorite", "text", "limit"]
        guard Set(args.object.keys).isSubset(of: allowed), args["text"].string.isEmpty else { throw SourceFailure.unsupported }
        for key in ["album", "kind", "text"] where args[key] != .null {
            guard case .string = args[key] else { throw SourceFailure.invalid }
        }
        for key in ["from", "to"] where args[key] != .null {
            guard parseTimestamp(args[key].string) != nil else { throw SourceFailure.invalid }
        }
        if args["favorite"] != .null, case .bool = args["favorite"] {} else if args["favorite"] != .null { throw SourceFailure.invalid }
        let rawLimit = args["limit"] == .null ? 100 : args["limit"].number
        guard rawLimit.isFinite, rawLimit.rounded() == rawLimit, rawLimit > 0, rawLimit <= 1000 else { throw SourceFailure.invalid }
        let limit = Int(rawLimit)
        let snapshot = try await snapshot()
        let rows = snapshot.filter { row in
            guard row["collection"].string == "photos.asset" else { return false }
            let data = row["data"]
            if args["favorite"] != .null && data["favorite"] != args["favorite"] { return false }
            if !args["kind"].string.isEmpty && data["kind"] != args["kind"] { return false }
            if !args["album"].string.isEmpty && !data["albums"].array.contains(args["album"]) { return false }
            for key in ["from", "to"] where args[key] != .null {
                guard let created = parseTimestamp(data["created"].string), let bound = parseTimestamp(args[key].string) else { return false }
                if key == "from" && created < bound || key == "to" && created > bound { return false }
            }
            return true
        }
        return .object(["items": .array(rows.prefix(limit).map { $0["data"] }), "truncated": .bool(rows.count > limit)])
    }
    public func blob(_ operation: String, _ args: JSON) async throws -> JSON {
        try Task.checkCancellation()
        guard activeBlobRequests < 2 else { throw SourceFailure.unavailable }
        activeBlobRequests += 1
        defer { activeBlobRequests -= 1 }
        guard operation == "photos.original" || operation == "photos.thumbnail", Set(args.object.keys) == ["asset"], !args["asset"].string.isEmpty else { throw SourceFailure.invalid }
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        let bytes: Data
        if fixture != nil {
            guard let value = fixtureBlobs[operation + "/" + args["asset"].string], !value.isEmpty, value.count <= 16 << 20 else { throw SourceFailure.unavailable }
            bytes = value
        } else {
            let result = PHAsset.fetchAssets(withLocalIdentifiers: [args["asset"].string], options: nil)
            guard let asset = result.firstObject else { throw SourceFailure.unavailable }
            if operation == "photos.original" { bytes = try await originalPhotoData(asset) } else { bytes = try await thumbnailPhotoData(asset) }
        }
        try Task.checkCancellation()
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        blobs = blobs.filter { $0.value.1 > Date() }
        guard blobs.values.reduce(0, { $0 + $1.0.count }) + bytes.count <= 64 << 20 else { throw SourceFailure.unavailable }
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        let handle = "photos-blob:" + UUID().uuidString
        blobs[handle] = (bytes, Date().addingTimeInterval(60), digest, args["asset"].string)
        return .object(["digest": .string(digest), "size": .number(Double(bytes.count)), "handle": .string(handle)])
    }
    public func readBlob(_ handle: String, offset: Int64, length: Int) async throws -> JSON {
        guard await open().state == "ready" else { throw SourceFailure.permissionMissing }
        blobs = blobs.filter { $0.value.1 > Date() }
        guard let entry = blobs[handle], offset >= 0, offset <= Int64(entry.0.count), length > 0, length <= 131072 else { throw SourceFailure.unavailable }
        if fixture == nil {
            guard PHAsset.fetchAssets(withLocalIdentifiers: [entry.3], options: nil).count == 1 else { throw SourceFailure.permissionMissing }
        }
        let start = Int(offset)
        let end = min(entry.0.count, start + length)
        let data = entry.0.subdata(in: start..<end)
        let digest = entry.2
        return .object(["handle": .string(handle), "offset": .number(Double(offset)), "data": .string(data.base64EncodedString()), "digest": .string(digest), "eof": .bool(end == entry.0.count)])
    }
    var importedAssets: Set<String> = []
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
