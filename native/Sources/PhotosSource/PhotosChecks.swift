#if LAB
import Foundation
import SourceSupport

public enum PhotosChecks {
    static func require(_ condition: @autoclosure () throws -> Bool) throws {
        guard try condition() else { throw SourceFailure.corrupt }
    }

    static func rejects<T>(_ action: @autoclosure () throws -> T) throws {
        do { _ = try action() } catch { return }
        throw SourceFailure.corrupt
    }

    public static func run() throws {
        try testPhotoKitFetchIsBoundedBeforeEnumeration()
        try testCursorPredicatesPreserveTiesAndUndatedAssets()
        try testTransferLifetimeUsesIdleAndHardMonotonicDeadlines()
        try testStrictBoundaries()
        try testStreamedOriginalAndBoundedRead()
        try testMaximumReadFitsNativeQueryFrame()
        try testCapacityAndFailureRemovePartialFiles()
        try testPreviewChecks()
    }

    static func testPhotoKitFetchIsBoundedBeforeEnumeration() throws {
        for limit in [1, 50, 200] {
            try require(photoSearchOptions(limit: limit, locationFiltered: false).fetchLimit == limit + 1)
            try require(photoSearchOptions(limit: limit, locationFiltered: true).fetchLimit == 1001)
            try require(photoSearchOptions(limit: limit, locationFiltered: false).sortDescriptors?.map(\.key) == ["creationDate"])
        }
    }

    static func testCursorPredicatesPreserveTiesAndUndatedAssets() throws {
        let date = Date(timeIntervalSinceReferenceDate: 800_000_000.123456)
        let fixtures: [NSDictionary] = [
            ["localIdentifier": "a", "creationDate": date.addingTimeInterval(1)],
            ["localIdentifier": "a", "creationDate": date],
            ["localIdentifier": "b", "creationDate": date],
            ["localIdentifier": "c", "creationDate": date],
            ["localIdentifier": "a", "creationDate": date.addingTimeInterval(-1)],
            ["localIdentifier": "a"],
            ["localIdentifier": "b"]
        ]
        for limit in [1, 2, 200] {
            var cursor: PhotoSearchCursor?
            var seen: [NSDictionary] = []
            var excluded: [NSDictionary] = []
            var boundary: Date?
            for iteration in 0...fixtures.count {
                let options = photoSearchOptions(limit: limit, locationFiltered: false, cursor: cursor, excluded: excluded)
                let candidates = fixtures.filter { options.predicate?.evaluate(with: $0) ?? true }.sorted {
                    let left = $0["creationDate"] as? Date
                    let right = $1["creationDate"] as? Date
                    if left != right { return (left ?? .distantPast) > (right ?? .distantPast) }
                    let leftID = $0["localIdentifier"] as? String ?? ""
                    let rightID = $1["localIdentifier"] as? String ?? ""
                    return iteration % 2 == 0 ? leftID < rightID : leftID > rightID
                }
                let fetched = Array(candidates.prefix(options.fetchLimit))
                let page = Array(fetched.prefix(limit))
                seen.append(contentsOf: page)
                for value in page {
                    let date = value["creationDate"] as? Date
                    if date != boundary { excluded.removeAll(); boundary = date }
                    excluded.append(value)
                }
                if fetched.count <= page.count { break }
                guard let last = page.last, let id = last["localIdentifier"] as? String else { throw SourceFailure.corrupt }
                cursor = try PhotoSearchCursor.decode(PhotoSearchCursor(date: last["creationDate"] as? Date, id: id).encode())
            }
            try require(seen.count == fixtures.count && Set(seen) == Set(fixtures))
        }
        let boundary = try PhotoSearchCursor.decode(PhotoSearchCursor(date: date, id: "b").encode())
        try require(boundary.date == date && boundary.id == "b")
        let options = photoSearchOptions(limit: 1, locationFiltered: true, cursor: boundary, excluded: Array(fixtures[1...2]))
        try require(fixtures.filter { options.predicate?.evaluate(with: $0) ?? false } == Array(fixtures[3...]))
        try rejects(try PhotoSearchCursor.decode("not-base64"))
        try rejects(try PhotoSearchCursor.decode(Data("{}".utf8).base64EncodedString()))
        try rejects(try PhotoSearchCursor.decode(PhotoSearchCursor(date: nil, id: "").encode()))
        try rejects(try PhotoSearchCursor.decode(String(repeating: "A", count: 2049)))
    }

    static func testTransferLifetimeUsesIdleAndHardMonotonicDeadlines() throws {
        let start = ContinuousClock().now
        var lifetime = TransferLifetime(started: start, touched: start)
        try require(!lifetime.expired(at: start.advanced(by: .seconds(119))))
        try require(lifetime.expired(at: start.advanced(by: .seconds(120))))
        lifetime.touched = start.advanced(by: .seconds(86399))
        try require(lifetime.expired(at: start.advanced(by: .seconds(86400))))
    }

    static func testStrictBoundaries() throws {
        try rejects(try fields(.object(["_principal": .string("caller")]), ["asset_id"]))
        try rejects(try fields(.array([]), []))
        try rejects(try integer(.object(["offset": .number(-1)]), "offset", default: 0, maximum: 100))
        try rejects(try integer(.object(["limit": .number(200.5)]), "limit", default: 50, maximum: 200))
        try rejects(try boolean(.object(["favorite": .string("true")]), "favorite", default: false))
        try rejects(try boolean(.object([:]), "favorite", default: false, required: true))
        try rejects(try string(.object(["title": .string("   ")]), "title"))
        try require(try string(.object(["title": .string(String(repeating: "é", count: 256))]), "title", maximum: 256) == String(repeating: "é", count: 256))
        try rejects(try string(.object(["title": .string(String(repeating: "é", count: 257))]), "title", maximum: 256))
        try require(try integer(.object(["length": .number(524288)]), "length", default: 1, maximum: 524288) == 524288)
        try require(try integer(.object(["offset": .number(68_719_476_736)]), "offset", default: 0, maximum: 64 << 30) == 64 << 30)
    }

    static func testStreamedOriginalAndBoundedRead() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-photos-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("resource")
        let download = try PhotoDownload(url: file, maximum: 128 << 20)
        try require(!FileManager.default.fileExists(atPath: file.path))
        for index in 0..<128 {
            download.append(Data(repeating: UInt8(index), count: 1 << 20))
        }
        download.finish(failed: false)
        let response = try download.read(id: "synthetic", offset: (128 << 20) - 5, length: 512 << 10)
        try require(response["size"].unsigned == 128 << 20)
        try require(response["digest"].string == "ec5223559dc61ecef215c97e5b6c1a0fad4cab8530aed3de1608c6cd47892ef9")
        try require(Data(base64Encoded: response["bytes"].string) == Data(repeating: 127, count: 5))
        try require(response["eof"] == .bool(true))
        try rejects(try download.read(id: "synthetic", offset: (128 << 20) + 1, length: 1))
        let middle = try download.read(id: "synthetic", offset: (64 << 20) - 2, length: 4)
        try require(Data(base64Encoded: middle["bytes"].string) == Data([63, 63, 64, 64]))
        try require(middle["eof"] == .bool(false))
        try require(!FileManager.default.fileExists(atPath: file.path))
        download.cancel()
        try require(!FileManager.default.fileExists(atPath: file.path))
        try rejects(try download.read(id: "synthetic", offset: 0, length: 1))
    }

    static func testMaximumReadFitsNativeQueryFrame() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-photos-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let download = try PhotoDownload(url: directory.appendingPathComponent("resource"), maximum: 512 << 10)
        download.append(Data(repeating: 255, count: 512 << 10))
        download.finish(failed: false)
        var offset = 0
        while offset < 512 << 10 {
            let response = try download.read(id: UUID().uuidString, offset: offset, length: 512 << 10)
            let frame: JSON = .object(["t": .string("QueryResult"), "id": .string(UUID().uuidString), "result_json": response, "blobs": .array([])])
            try require(try JSONEncoder().encode(frame).count <= 256 << 10)
            let bytes = Data(base64Encoded: response["bytes"].string) ?? Data()
            try require(!bytes.isEmpty && bytes == Data(repeating: 255, count: bytes.count))
            offset += bytes.count
            try require(response["eof"] == .bool(offset == 512 << 10))
        }
        try require(offset == 512 << 10)
    }

    static func testCapacityAndFailureRemovePartialFiles() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-photos-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let empty = try PhotoDownload(url: directory.appendingPathComponent("empty"), maximum: 4)
        empty.finish(failed: false)
        try rejects(try empty.read(id: "synthetic", offset: 0, length: 1))
        for fail in [false, true] {
            let file = directory.appendingPathComponent(UUID().uuidString)
            let download = try PhotoDownload(url: file, maximum: 4)
            download.append(Data([1, 2, 3, 4]))
            if fail {
                download.finish(failed: true)
            } else {
                download.append(Data([5]))
                download.finish(failed: false)
            }
            try require(!FileManager.default.fileExists(atPath: file.path))
            try rejects(try download.read(id: "synthetic", offset: 0, length: 1))
        }
    }
}
#endif
