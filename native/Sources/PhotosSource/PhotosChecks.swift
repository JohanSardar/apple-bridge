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
        try testTransferLifetimeUsesIdleAndHardMonotonicDeadlines()
        try testStrictBoundaries()
        try testStreamedOriginalAndBoundedRead()
        try testCapacityAndFailureRemovePartialFiles()
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
