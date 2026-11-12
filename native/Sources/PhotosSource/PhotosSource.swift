import Foundation
import CryptoKit
@preconcurrency import Photos
import CoreLocation
import SourceSupport

public actor PhotosSource: Source {
    public nonisolated let name = "photos"
    public static let operations = ["photos.search", "photos.places", "photos.get", "photos.albums", "photos.preview", "photos.contact_sheet", "photos.fetch", "photos.release", "photos.favorite", "photos.album_create", "photos.album_rename", "photos.album_add", "photos.album_remove", "photos.album_delete"]
    private let transfers: PhotoTransfers
    private let places = PhotoPlaces()
    private var searches: [String: PhotoSearchContinuation] = [:]
    private var searchExpiryTask: Task<Void, Never>?

    public init(home: URL) { transfers = PhotoTransfers(home: home) }

    public func status() async -> JSON? {
        let state = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        if state != .authorized && state != .limited {
            searches.removeAll()
            await transfers.cancelAll()
        }
        return .object(["authorized": .bool(state == .authorized || state == .limited), "limited": .bool(state == .limited)])
    }

    public func query(_ operation: String, args input: JSON) async throws -> JSON {
        let authorization = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard authorization == .authorized || authorization == .limited else {
            searches.removeAll()
            await transfers.cancelAll()
            throw SourceFailure.permissionMissing
        }
        try Task.checkCancellation()
        guard case .object = input else { throw SourceFailure.invalid }
        if operation == "photos.fetch" { return try await transfers.fetch(input) }
        if operation == "photos.release" { return try await transfers.release(input) }
        var publicFields = input.object
        publicFields.removeValue(forKey: "_principal")
        publicFields.removeValue(forKey: "_request")
        let args = JSON.object(publicFields)
        switch operation {
        case "photos.search": return try await search(args)
        case "photos.places": return try await placeSummary(args)
        case "photos.get":
            try fields(args, ["asset_id"])
            let value = try asset(try string(args, "asset_id"))
            let place = await named([value], budget: .seconds(2))[value.localIdentifier]
            var result = try details(value).object
            if let place, case .object(var location) = result["location"] {
                location["place"] = .string(place.name)
                result["location"] = .object(location)
                result["place"] = .string(place.name)
            }
            return .object(result)
        case "photos.albums": return try albums(args)
        case "photos.preview": return try await photoPreview(args)
        case "photos.contact_sheet": return try await photoContactSheet(args)
        default: return try await change(operation, args: args)
        }
    }

    private func search(_ args: JSON) async throws -> JSON {
        try fields(args, ["start", "end", "type", "album_id", "favorite", "include_hidden", "near", "radius_m", "cursor", "limit"])
        let limit = try integer(args, "limit", default: 50, maximum: 200, minimum: 1)
        let cursor = args["cursor"] == .null ? nil : try PhotoSearchCursor.decode(string(args, "cursor", maximum: 2048))
        let now = ContinuousClock().now
        searches = searches.filter { $0.value.started.duration(to: now) < .seconds(600) }
        var filters = args.object
        filters.removeValue(forKey: "cursor")
        let continuation: PhotoSearchContinuation?
        if let cursor {
            guard let saved = searches[cursor.id], saved.filters == .object(filters) else { throw SourceFailure.expired }
            continuation = saved
        } else { continuation = nil }
        let options = photoSearchOptions(limit: limit, locationFiltered: args["near"] != .null, cursor: continuation?.cursor, excluded: continuation?.excluded ?? [])
        options.includeHiddenAssets = try boolean(args, "include_hidden", default: false)
        var predicates = options.predicate.map { [$0] } ?? []
        for (key, relation) in [("start", ">="), ("end", "<")] where args[key] != .null {
            guard let date = parseTimestamp(try string(args, key)) else { throw SourceFailure.invalid }
            predicates.append(NSPredicate(format: "creationDate \(relation) %@", date as NSDate))
        }
        if args["start"] != .null && args["end"] != .null {
            guard let start = parseTimestamp(args["start"].string), let end = parseTimestamp(args["end"].string), start < end else { throw SourceFailure.invalid }
        }
        if args["favorite"] != .null {
            predicates.append(NSPredicate(format: "favorite == %@", NSNumber(value: try boolean(args, "favorite", default: false))))
        }
        let subtypes: [String: PHAssetMediaSubtype] = ["live_photo": .photoLive, "screenshot": .photoScreenshot, "panorama": .photoPanorama, "hdr": .photoHDR, "depth": .photoDepthEffect, "slow_motion": .videoHighFrameRate, "timelapse": .videoTimelapse]
        if args["type"] != .null {
            let kind = try string(args, "type")
            if let subtype = subtypes[kind] {
                predicates.append(NSPredicate(format: "(mediaSubtypes & %d) != 0", subtype.rawValue))
            } else {
                guard let type = ["image": PHAssetMediaType.image, "video": .video, "audio": .audio][kind] else { throw SourceFailure.invalid }
                predicates.append(NSPredicate(format: "mediaType == %d", type.rawValue))
            }
        }
        let near: CLLocation?
        let radius: Double
        if args["near"] != .null {
            try fields(args["near"], ["lat", "lon"])
            let lat = args["near"]["lat"].number
            let lon = args["near"]["lon"].number
            radius = args["radius_m"] == .null ? 1000 : args["radius_m"].number
            guard args["near"]["lat"] == .number(lat), args["near"]["lon"] == .number(lon), abs(lat) <= 90, abs(lon) <= 180, radius > 0, radius <= 100_000, radius.isFinite else { throw SourceFailure.invalid }
            near = CLLocation(latitude: lat, longitude: lon)
        } else {
            guard args["radius_m"] == .null else { throw SourceFailure.invalid }
            near = nil
            radius = 0
        }
        options.predicate = NSCompoundPredicate(andPredicateWithSubpredicates: predicates)
        try Task.checkCancellation()
        let results: PHFetchResult<PHAsset>
        if args["album_id"] != .null {
            results = PHAsset.fetchAssets(in: try album(string(args, "album_id")), options: options)
        } else {
            results = PHAsset.fetchAssets(with: options)
        }
        try Task.checkCancellation()
        var found: [PHAsset] = []
        var index = 0
        let stop = min(results.count, options.fetchLimit - 1)
        var boundary = continuation?.cursor.date
        var excluded = continuation?.excluded ?? []
        while index < stop && found.count < limit {
            try Task.checkCancellation()
            let value = results.object(at: index)
            index += 1
            if value.creationDate != boundary { excluded.removeAll(); boundary = value.creationDate }
            excluded.append(value)
            if let near {
                guard let location = value.location, near.distance(from: location) <= radius else { continue }
            }
            found.append(value)
        }
        var next: JSON = .null
        if index < results.count {
            guard excluded.count <= 131_072 else { throw SourceFailure.unavailable }
            while searches.count >= 128 || searches.values.reduce(0, { $0 + $1.excluded.count }) + excluded.count > 131_072 {
                guard let oldest = searches.min(by: { $0.value.started < $1.value.started })?.key else { throw SourceFailure.unavailable }
                searches.removeValue(forKey: oldest)
            }
            let cursor = PhotoSearchCursor(date: boundary, id: UUID().uuidString)
            searches[cursor.id] = PhotoSearchContinuation(cursor: cursor, excluded: excluded, filters: .object(filters), started: now)
            if searchExpiryTask == nil {
                searchExpiryTask = Task {
                    while !self.searches.isEmpty {
                        try? await Task.sleep(for: .seconds(15))
                        let now = ContinuousClock().now
                        self.searches = self.searches.filter { $0.value.started.duration(to: now) < .seconds(600) }
                    }
                    self.searchExpiryTask = nil
                }
            }
            next = .string(try cursor.encode())
        }
        let names = await named(found, budget: .seconds(3))
        let rows: [JSON] = found.map { value in
            var row = summary(value).object
            if let place = names[value.localIdentifier] { row["place"] = .string(place.name) }
            return .object(row)
        }
        return .object(["assets": .array(rows), "examined_count": .number(Double(index)), "next_cursor": next, "limited": .bool(PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited)])
    }

    private func named(_ assets: [PHAsset], budget: Duration) async -> [String: PhotoPlace] {
        var spots: [(PlaceCell, CLLocation)] = []
        var seen: Set<PlaceCell> = []
        var cells: [(String, PlaceCell)] = []
        for value in assets {
            guard let cell = PlaceCell(value.location), let location = value.location else { continue }
            cells.append((value.localIdentifier, cell))
            if seen.insert(cell).inserted { spots.append((cell, location)) }
        }
        _ = await places.resolve(spots, budget: budget)
        var result: [String: PhotoPlace] = [:]
        for (id, cell) in cells {
            if let place = await places.cached(cell) { result[id] = place }
        }
        return result
    }

    private func placeSummary(_ args: JSON) async throws -> JSON {
        try fields(args, ["start", "end", "include_hidden", "include_screenshots"])
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: true)]
        options.includeHiddenAssets = try boolean(args, "include_hidden", default: false)
        var predicates: [NSPredicate] = []
        var bounds: [Date] = []
        for (key, relation) in [("start", ">="), ("end", "<")] where args[key] != .null {
            guard let date = parseTimestamp(try string(args, key)) else { throw SourceFailure.invalid }
            bounds.append(date)
            predicates.append(NSPredicate(format: "creationDate \(relation) %@", date as NSDate))
        }
        if bounds.count == 2, bounds[0] >= bounds[1] { throw SourceFailure.invalid }
        if try !boolean(args, "include_screenshots", default: false) {
            predicates.append(NSPredicate(format: "(mediaSubtypes & %d) == 0", PHAssetMediaSubtype.photoScreenshot.rawValue))
        }
        options.predicate = NSCompoundPredicate(andPredicateWithSubpredicates: predicates)
        let results = PHAsset.fetchAssets(with: options)
        guard results.count <= 500_000 else { throw SourceFailure.unavailable }
        var located: [(PHAsset, PlaceCell)] = []
        var spots: [(PlaceCell, CLLocation)] = []
        var seen: Set<PlaceCell> = []
        var unlocated = 0
        for index in 0..<results.count {
            if index % 1000 == 0 { try Task.checkCancellation() }
            let value = results.object(at: index)
            guard let location = value.location, let cell = PlaceCell(location) else { unlocated += 1; continue }
            located.append((value, cell))
            if seen.insert(cell).inserted { spots.append((cell, location)) }
        }
        let complete = await places.resolve(spots, budget: .seconds(25))
        var groups: [String: PlaceGroup] = [:]
        var order: [String] = []
        var unnamed = 0
        for (value, cell) in located {
            guard let place = await places.cached(cell) else { unnamed += 1; continue }
            if groups[place.name] == nil { order.append(place.name); groups[place.name] = PlaceGroup(place: place) }
            groups[place.name]?.add(value)
        }
        let rows = order.compactMap { groups[$0] }
        func rollup(_ key: (PhotoPlace) -> String?) -> JSON {
            var totals: [String: PlaceGroup] = [:]
            var keys: [String] = []
            for group in rows {
                guard let name = key(group.place) else { continue }
                if totals[name] == nil { keys.append(name); totals[name] = PlaceGroup(place: group.place) }
                totals[name]?.merge(group)
            }
            return .array(keys.compactMap { name in totals[name].map { $0.rollup(name) } })
        }
        return .object([
            "places": .array(rows.map(\.json)),
            "countries": rollup(\.country),
            "us_states": rollup(\.state),
            "located_count": .number(Double(located.count)),
            "unlocated_count": .number(Double(unlocated)),
            "unnamed_count": .number(Double(unnamed)),
            "complete": .bool(complete),
            "limited": .bool(PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited),
        ])
    }

    private func albums(_ args: JSON) throws -> JSON {
        try fields(args, ["offset", "limit", "smart"])
        let offset = try integer(args, "offset", default: 0, maximum: 100_000_000)
        let limit = try integer(args, "limit", default: 50, maximum: 200, minimum: 1)
        let smart = try boolean(args, "smart", default: false)
        let results = PHAssetCollection.fetchAssetCollections(with: smart ? .smartAlbum : .album, subtype: .any, options: nil)
        var rows: [JSON] = []
        let end = min(results.count, min(offset, results.count) + limit)
        for index in min(offset, results.count)..<end {
            try Task.checkCancellation()
            let value = results.object(at: index)
            rows.append(.object(["id": .string(value.localIdentifier), "title": .string(value.localizedTitle ?? ""), "type": .number(Double(value.assetCollectionType.rawValue)), "subtype": .number(Double(value.assetCollectionSubtype.rawValue)), "can_add": .bool(value.canPerform(.addContent)), "can_remove": .bool(value.canPerform(.removeContent)), "can_delete": .bool(value.canPerform(.delete))]))
        }
        return .object(["albums": .array(rows), "count": .number(Double(results.count)), "next_offset": end < results.count ? .number(Double(end)) : .null, "limited": .bool(PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited)])
    }

    private func summary(_ value: PHAsset) -> JSON {
        .object(["id": .string(value.localIdentifier), "type": .string([PHAssetMediaType.image: "image", .video: "video", .audio: "audio"][value.mediaType] ?? "unknown"), "subtypes": .number(Double(value.mediaSubtypes.rawValue)), "created_at": value.creationDate.map { .string(timestamp($0)) } ?? .null, "modified_at": value.modificationDate.map { .string(timestamp($0)) } ?? .null, "width": .number(Double(value.pixelWidth)), "height": .number(Double(value.pixelHeight)), "duration_s": .number(value.duration), "favorite": .bool(value.isFavorite), "hidden": .bool(value.isHidden), "burst_id": value.burstIdentifier.map(JSON.string) ?? .null])
    }

    private func details(_ value: PHAsset) throws -> JSON {
        var result = summary(value).object
        if let location = value.location {
            result["location"] = .object(["lat": .number(location.coordinate.latitude), "lon": .number(location.coordinate.longitude), "altitude_m": .number(location.altitude), "accuracy_m": .number(location.horizontalAccuracy)])
        }
        result["resources"] = .array(try PHAssetResource.assetResources(for: value).enumerated().map { index, resource in
            .object(["index": .number(Double(index)), "type": .number(Double(resource.type.rawValue)), "filename": .string(resource.originalFilename), "uti": .string(resource.uniformTypeIdentifier), "resource_key": .string(try resourceKey(resource, asset: value))])
        })
        result["can_favorite"] = .bool(value.canPerform(.properties))
        result["source_type"] = .number(Double(value.sourceType.rawValue))
        result["burst_selection"] = .number(Double(value.burstSelectionTypes.rawValue))
        result["represents_burst"] = .bool(value.representsBurst)
        result["limited"] = .bool(PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited)
        return .object(result)
    }

    private func change(_ operation: String, args: JSON) async throws -> JSON {
        let allowed: Set<String>
        switch operation {
        case "photos.favorite": allowed = ["asset_id", "favorite"]
        case "photos.album_create": allowed = ["title"]
        case "photos.album_rename": allowed = ["album_id", "title"]
        case "photos.album_add", "photos.album_remove": allowed = ["album_id", "asset_ids"]
        case "photos.album_delete": allowed = ["album_id"]
        default: throw SourceFailure.unsupported
        }
        try fields(args, allowed)
        for key in allowed {
            guard args[key] != .null else { throw SourceFailure.invalid }
        }
        let title = args["title"] == .null ? "" : try string(args, "title", maximum: 256)
        let target = args["album_id"] == .null ? nil : try album(string(args, "album_id"))
        let photo = args["asset_id"] == .null ? nil : try asset(string(args, "asset_id"))
        let favorite = operation == "photos.favorite" ? try boolean(args, "favorite", default: false, required: true) : false
        var members: [PHAsset] = []
        if operation == "photos.album_add" || operation == "photos.album_remove" {
            guard case .array(let ids) = args["asset_ids"], !ids.isEmpty, ids.count <= 200 else { throw SourceFailure.invalid }
            members = try ids.map { guard case .string(let id) = $0 else { throw SourceFailure.invalid }; return try asset(id) }
            guard Set(members.map(\.localIdentifier)).count == members.count else { throw SourceFailure.invalid }
        }
        if let target {
            guard target.assetCollectionType == .album, target.assetCollectionSubtype == .albumRegular else { throw SourceFailure.unsupported }
            let editing: PHCollectionEditOperation = operation == "photos.album_delete" ? .delete : operation == "photos.album_add" ? .addContent : operation == "photos.album_remove" ? .removeContent : .rename
            guard target.canPerform(editing) else { throw SourceFailure.unsupported }
        }
        if let photo { guard photo.canPerform(.properties) else { throw SourceFailure.unsupported } }
        let selectedMembers = members
        let created = CreatedAlbum()
        try Task.checkCancellation()
        try await PHPhotoLibrary.shared().performChanges {
            switch operation {
            case "photos.favorite":
                if let photo { PHAssetChangeRequest(for: photo).isFavorite = favorite }
            case "photos.album_create":
                let request = PHAssetCollectionChangeRequest.creationRequestForAssetCollection(withTitle: title)
                created.set(request.placeholderForCreatedAssetCollection.localIdentifier)
            case "photos.album_rename":
                if let target { PHAssetCollectionChangeRequest(for: target)?.title = title }
            case "photos.album_add":
                if let target { PHAssetCollectionChangeRequest(for: target)?.addAssets(selectedMembers as NSArray) }
            case "photos.album_remove":
                if let target { PHAssetCollectionChangeRequest(for: target)?.removeAssets(selectedMembers as NSArray) }
            case "photos.album_delete":
                if let target { PHAssetCollectionChangeRequest.deleteAssetCollections([target] as NSArray) }
            default: break
            }
        }
        if let photo {
            guard try asset(photo.localIdentifier).isFavorite == favorite else { throw SourceFailure.conflict }
        }
        if operation == "photos.album_delete", let target {
            guard PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [target.localIdentifier], options: nil).count == 0 else { throw SourceFailure.conflict }
        }
        if operation == "photos.album_rename", let target {
            guard try album(target.localIdentifier).localizedTitle == title else { throw SourceFailure.conflict }
        }
        if let target, !members.isEmpty {
            let options = PHFetchOptions()
            options.includeHiddenAssets = true
            options.includeAllBurstAssets = true
            let contents = PHAsset.fetchAssets(in: try album(target.localIdentifier), options: options)
            for member in members {
                guard (contents.index(of: member) != NSNotFound) == (operation == "photos.album_add") else { throw SourceFailure.conflict }
            }
        }
        if operation == "photos.album_create" {
            let id = created.get()
            guard !id.isEmpty, try album(id).localizedTitle == title else { throw SourceFailure.conflict }
            return .object(["completed": .bool(true), "album_id": .string(id)])
        }
        return .object(["completed": .bool(true), "affected_count": .number(Double(max(1, members.count)))])
    }
}

private struct PlaceGroup {
    let place: PhotoPlace
    var count = 0
    var first: Date?
    var last: Date?
    var places: [String] = []
    var pick: PHAsset?
    var pickRank = -1

    mutating func add(_ value: PHAsset) {
        count += 1
        if let date = value.creationDate {
            first = first.map { min($0, date) } ?? date
            last = last.map { max($0, date) } ?? date
        }
        let rank = (value.isFavorite ? 2 : 0) + (value.mediaType == .image ? 1 : 0)
        if rank > pickRank { pick = value; pickRank = rank }
    }

    mutating func merge(_ other: PlaceGroup) {
        count += other.count
        first = [first, other.first].compactMap { $0 }.min()
        last = [last, other.last].compactMap { $0 }.max()
        if !places.contains(other.place.name) { places.append(other.place.name) }
    }

    var json: JSON {
        var row = place.json.object
        row["photo_count"] = .number(Double(count))
        row["first_taken_at"] = first.map { .string(timestamp($0)) } ?? .null
        row["last_taken_at"] = last.map { .string(timestamp($0)) } ?? .null
        row["asset_id"] = pick.map { .string($0.localIdentifier) } ?? .null
        return .object(row)
    }

    func rollup(_ name: String) -> JSON {
        .object(["name": .string(name), "photo_count": .number(Double(count)), "first_taken_at": first.map { .string(timestamp($0)) } ?? .null, "last_taken_at": last.map { .string(timestamp($0)) } ?? .null, "places": .array(places.map(JSON.string))])
    }
}

struct PhotoSearchCursor: Codable {
    let date: Date?
    let id: String

    func encode() throws -> String {
        try JSONEncoder().encode(self).base64EncodedString()
    }

    static func decode(_ text: String) throws -> Self {
        guard text.utf8.count <= 2048, let data = Data(base64Encoded: text),
              let cursor = try? JSONDecoder().decode(Self.self, from: data),
              !cursor.id.isEmpty, cursor.id.utf8.count <= 1024,
              cursor.date?.timeIntervalSinceReferenceDate.isFinite != false else { throw SourceFailure.invalid }
        return cursor
    }
}

private struct PhotoSearchContinuation {
    let cursor: PhotoSearchCursor
    let excluded: [PHAsset]
    let filters: JSON
    let started: ContinuousClock.Instant
}

func photoSearchOptions(limit: Int, locationFiltered: Bool, cursor: PhotoSearchCursor? = nil, excluded: [Any] = []) -> PHFetchOptions {
    let options = PHFetchOptions()
    options.fetchLimit = (locationFiltered ? 1000 : limit) + 1
    options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
    options.includeAllBurstAssets = true
    if let cursor {
        if let date = cursor.date {
            options.predicate = NSPredicate(format: "creationDate < %@ OR (creationDate == %@ AND NOT (SELF IN %@)) OR creationDate == nil", date as NSDate, date as NSDate, excluded)
        } else {
            options.predicate = NSPredicate(format: "creationDate == nil AND NOT (SELF IN %@)", excluded)
        }
    }
    return options
}

func resourceKey(_ resource: PHAssetResource, asset: PHAsset) throws -> String {
    let identity = [asset.localIdentifier, asset.modificationDate.map(timestamp) ?? "", String(resource.type.rawValue), resource.originalFilename, resource.uniformTypeIdentifier]
    return SHA256.hash(data: try JSONEncoder().encode(identity)).map { String(format: "%02x", $0) }.joined()
}

private final class CreatedAlbum: @unchecked Sendable {
    private let lock = NSLock()
    private var id = ""
    func set(_ value: String) { lock.lock(); id = value; lock.unlock() }
    func get() -> String { lock.lock(); defer { lock.unlock() }; return id }
}

func fields(_ value: JSON, _ allowed: Set<String>) throws {
    guard case .object = value, Set(value.object.keys).isSubset(of: allowed) else { throw SourceFailure.invalid }
}

func string(_ value: JSON, _ key: String, maximum: Int = 1024) throws -> String {
    guard case .string(let text) = value[key], !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.unicodeScalars.count <= maximum else { throw SourceFailure.invalid }
    return text
}

func integer(_ value: JSON, _ key: String, default fallback: Int, maximum: Int, minimum: Int = 0) throws -> Int {
    if value[key] == .null { return fallback }
    let number = value[key].number
    guard value[key] == .number(number), number.isFinite, number.rounded() == number, number >= Double(minimum), number <= Double(maximum) else { throw SourceFailure.invalid }
    return Int(number)
}

func boolean(_ value: JSON, _ key: String, default fallback: Bool, required: Bool = false) throws -> Bool {
    if value[key] == .null && !required { return fallback }
    guard case .bool(let result) = value[key] else { throw SourceFailure.invalid }
    return result
}

func asset(_ id: String) throws -> PHAsset {
    let options = PHFetchOptions()
    options.includeHiddenAssets = true
    options.includeAllBurstAssets = true
    guard !id.isEmpty, id.utf8.count <= 1024, let value = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: options).firstObject else { throw SourceFailure.invalid }
    return value
}

func album(_ id: String) throws -> PHAssetCollection {
    guard !id.isEmpty, id.utf8.count <= 1024, let value = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [id], options: nil).firstObject else { throw SourceFailure.invalid }
    return value
}
