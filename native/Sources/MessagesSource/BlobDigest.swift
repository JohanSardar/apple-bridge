import Foundation
import CryptoKit
import Darwin

enum BlobDigest {
    static func isDataless(_ url: URL) -> Bool {
        var metadata = stat()
        return Darwin.lstat(url.path, &metadata) == 0 && metadata.st_flags & UInt32(SF_DATALESS) != 0
    }
    static func file(_ url: URL) throws -> String {
        guard !isDataless(url) else { throw ProbeFailure.unavailable }
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        var digest = SHA256()
        while let bytes = try file.read(upToCount: 128 * 1024), !bytes.isEmpty { try Task.checkCancellation(); digest.update(data: bytes) }
        return digest.finalize().map { String(format: "%02x", $0) }.joined()
    }
}
