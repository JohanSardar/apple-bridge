import Foundation
import XCTest
import SourceSupport
@testable import PhotosSource

final class PhotosTests: XCTestCase {
    func testTransferLifetimeUsesIdleAndHardMonotonicDeadlines() {
        let start = ContinuousClock().now
        var lifetime = TransferLifetime(started: start, touched: start)
        XCTAssertFalse(lifetime.expired(at: start.advanced(by: .seconds(119))))
        XCTAssertTrue(lifetime.expired(at: start.advanced(by: .seconds(120))))
        lifetime.touched = start.advanced(by: .seconds(86399))
        XCTAssertTrue(lifetime.expired(at: start.advanced(by: .seconds(86400))))
    }

    func testStrictBoundaries() throws {
        XCTAssertThrowsError(try fields(.object(["_principal": .string("caller")]), ["asset_id"]))
        XCTAssertThrowsError(try fields(.array([]), []))
        XCTAssertThrowsError(try integer(.object(["offset": .number(-1)]), "offset", default: 0, maximum: 100))
        XCTAssertThrowsError(try integer(.object(["limit": .number(200.5)]), "limit", default: 50, maximum: 200))
        XCTAssertThrowsError(try boolean(.object(["favorite": .string("true")]), "favorite", default: false))
        XCTAssertThrowsError(try boolean(.object([:]), "favorite", default: false, required: true))
        XCTAssertThrowsError(try string(.object(["title": .string("   ")]), "title"))
        XCTAssertEqual(try string(.object(["title": .string(String(repeating: "é", count: 256))]), "title", maximum: 256), String(repeating: "é", count: 256))
        XCTAssertThrowsError(try string(.object(["title": .string(String(repeating: "é", count: 257))]), "title", maximum: 256))
        XCTAssertEqual(try integer(.object(["length": .number(524288)]), "length", default: 1, maximum: 524288), 524288)
        XCTAssertEqual(try integer(.object(["offset": .number(68_719_476_736)]), "offset", default: 0, maximum: 64 << 30), 64 << 30)
    }

    func testStreamedOriginalAndBoundedRead() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-photos-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("resource")
        let download = try PhotoDownload(url: file, maximum: 128 << 20)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        for index in 0..<128 {
            download.append(Data(repeating: UInt8(index), count: 1 << 20))
        }
        download.finish(failed: false)
        let response = try download.read(id: "synthetic", offset: (128 << 20) - 5, length: 512 << 10)
        XCTAssertEqual(response["size"].unsigned, 128 << 20)
        XCTAssertEqual(response["digest"].string, "ec5223559dc61ecef215c97e5b6c1a0fad4cab8530aed3de1608c6cd47892ef9")
        XCTAssertEqual(Data(base64Encoded: response["bytes"].string), Data(repeating: 127, count: 5))
        XCTAssertEqual(response["eof"], .bool(true))
        XCTAssertThrowsError(try download.read(id: "synthetic", offset: (128 << 20) + 1, length: 1))
        let middle = try download.read(id: "synthetic", offset: (64 << 20) - 2, length: 4)
        XCTAssertEqual(Data(base64Encoded: middle["bytes"].string), Data([63, 63, 64, 64]))
        XCTAssertEqual(middle["eof"], .bool(false))
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        download.cancel()
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertThrowsError(try download.read(id: "synthetic", offset: 0, length: 1))
    }

    func testCapacityAndFailureRemovePartialFiles() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-photos-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let empty = try PhotoDownload(url: directory.appendingPathComponent("empty"), maximum: 4)
        empty.finish(failed: false)
        XCTAssertThrowsError(try empty.read(id: "synthetic", offset: 0, length: 1))
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
            XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
            XCTAssertThrowsError(try download.read(id: "synthetic", offset: 0, length: 1))
        }
    }
}
