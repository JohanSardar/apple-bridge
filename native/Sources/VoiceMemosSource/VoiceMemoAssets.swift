import Foundation
import CryptoKit
import Darwin
import SourceSupport

private struct VoiceMemoCopiedBlob: Sendable {
    let reference: JSON
    let file: URL
}

@MainActor
final class VoiceMemoAssets {
    let root: URL
    private var files: [String: URL] = [:]
    init(home: URL) throws {
        root = home.appendingPathComponent("voice-memo-blobs", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
    }
    deinit { try? FileManager.default.removeItem(at: root.deletingLastPathComponent()) }
    func stage(_ source: URL) async throws -> JSON {
        let root = self.root
        let job = Task.detached(priority: .utility) { try Self.copy(source, root: root) }
        return try await finish(job)
    }
    private func finish(_ job: Task<VoiceMemoCopiedBlob, Error>) async throws -> JSON {
        let copied = try await withTaskCancellationHandler(operation: { try await job.value }, onCancel: { job.cancel() })
        try Task.checkCancellation()
        files["voice-memo-blob:" + copied.reference["digest"].string] = copied.file
        return copied.reference
    }
    nonisolated private static func copy(_ source: URL, root: URL) throws -> VoiceMemoCopiedBlob {
        let fd = Darwin.open(source.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { throw SourceFailure.unavailable }
        let input = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? input.close() }
        var before = stat()
        guard fstat(fd, &before) == 0, before.st_mode & S_IFMT == S_IFREG else { throw SourceFailure.unavailable }
        let temporary = root.appendingPathComponent(UUID().uuidString)
        let outputFD = Darwin.open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard outputFD >= 0 else { throw SourceFailure.unavailable }
        let output = FileHandle(fileDescriptor: outputFD, closeOnDealloc: true)
        defer { try? output.close(); try? FileManager.default.removeItem(at: temporary) }
        var hash = SHA256(), size: Int64 = 0
        while let bytes = try input.read(upToCount: 131072), !bytes.isEmpty {
            try Task.checkCancellation()
            size += Int64(bytes.count)
            guard size <= before.st_size else { throw SourceFailure.conflict }
            hash.update(data: bytes)
            try output.write(contentsOf: bytes)
        }
        var after = stat()
        guard fstat(fd, &after) == 0, size == before.st_size, before.st_size == after.st_size,
              before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec,
              before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec else { throw SourceFailure.conflict }
        try output.synchronize()
        let digest = hash.finalize().map { String(format: "%02x", $0) }.joined()
        let destination = root.appendingPathComponent(digest)
        if !FileManager.default.fileExists(atPath: destination.path) { try FileManager.default.moveItem(at: temporary, to: destination) }
        return VoiceMemoCopiedBlob(reference: .object(["digest": .string(digest), "size": .number(Double(size)), "state": .string("present")]), file: destination)
    }
    func file(_ digest: String) -> URL? { files["voice-memo-blob:" + digest] }
    func handles(_ references: [JSON]) -> [String: String] {
        var result: [String: String] = [:]
        for ref in references where files["voice-memo-blob:" + ref["digest"].string] != nil { result[ref["digest"].string] = "voice-memo-blob:" + ref["digest"].string }
        return result
    }
    func read(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard let file = files[handle], offset >= 0, length > 0, length <= 131072 else { throw SourceFailure.unavailable }
        let fd = Darwin.open(file.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { throw SourceFailure.unavailable }
        let input = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? input.close() }
        let size = try input.seekToEnd()
        guard UInt64(offset) <= size else { throw SourceFailure.unavailable }
        try input.seek(toOffset: UInt64(offset))
        let bytes = try input.read(upToCount: min(length, Int(size - UInt64(offset)))) ?? Data()
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(String(handle.dropFirst("voice-memo-blob:".count))), "eof": .bool(UInt64(offset + bytes.count) == size)])
    }
}
