import Foundation
import CryptoKit
import Darwin

public struct StagedBlob: Sendable {
    public let digest: String
    public let size: Int64
    public let file: URL
    public var reference: JSON { .object(["digest": .string(digest), "size": .number(Double(size)), "state": .string("present")]) }
}

public struct BlobWriter: Sendable {
    public let directory: URL

    public init(directory: URL) throws {
        self.directory = directory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let info = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard info.isDirectory == true, info.isSymbolicLink != true else { throw SourceFailure.invalid }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
    }

    public func stage(_ bytes: Data) throws -> StagedBlob {
        var supplied = false
        return try write {
            guard !supplied else { return nil }
            supplied = true
            return bytes
        }
    }

    public func stage(_ source: URL) throws -> StagedBlob {
        let descriptor = Darwin.open(source.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        let input = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? input.close() }
        var before = stat()
        guard fstat(descriptor, &before) == 0, before.st_mode & S_IFMT == S_IFREG,
              before.st_flags & UInt32(SF_DATALESS) == 0 else { throw SourceFailure.unavailable }
        var size: Int64 = 0
        return try write {
            let bytes = try input.read(upToCount: 131072) ?? Data()
            size += Int64(bytes.count)
            guard size <= before.st_size else { throw SourceFailure.conflict }
            if !bytes.isEmpty { return bytes }
            var after = stat()
            guard fstat(descriptor, &after) == 0, size == before.st_size, before.st_size == after.st_size,
                  before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec, before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec,
                  before.st_ctimespec.tv_sec == after.st_ctimespec.tv_sec, before.st_ctimespec.tv_nsec == after.st_ctimespec.tv_nsec else { throw SourceFailure.conflict }
            return nil
        }
    }

    private func write(next: () throws -> Data?) throws -> StagedBlob {
        let temporary = directory.appendingPathComponent(UUID().uuidString)
        let descriptor = Darwin.open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        let output = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? output.close(); try? FileManager.default.removeItem(at: temporary) }
        var hash = SHA256(), size: Int64 = 0
        while let bytes = try next() {
            try Task.checkCancellation()
            hash.update(data: bytes)
            size += Int64(bytes.count)
            try output.write(contentsOf: bytes)
        }
        try Task.checkCancellation()
        try output.synchronize()
        let digest = hash.finalize().map { String(format: "%02x", $0) }.joined()
        let destination = directory.appendingPathComponent(digest)
        guard Darwin.rename(temporary.path, destination.path) == 0 else { throw SourceFailure.unavailable }
        return StagedBlob(digest: digest, size: size, file: destination)
    }
}
