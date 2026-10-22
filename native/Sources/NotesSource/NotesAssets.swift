import Foundation
import CryptoKit
import Darwin
import SourceSupport

private struct NotesCopiedBlob: Sendable {
    let reference: JSON
    let file: URL
}

@MainActor
final class NotesAssets {
    let root: URL
    private var files: [String: URL] = [:]
    init(home: URL) throws {
        root = home.appendingPathComponent("notes-blobs", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
    }
    func stage(_ source: URL) async throws -> JSON {
        let root = self.root
        let job = Task.detached(priority: .utility) { try Self.copy(source, root: root) }
        return try await finish(job)
    }
    private func finish(_ job: Task<NotesCopiedBlob, Error>) async throws -> JSON {
        let copied = try await withTaskCancellationHandler(operation: { try await job.value }, onCancel: { job.cancel() })
        try Task.checkCancellation()
        files["notes-blob:" + copied.reference["digest"].string] = copied.file
        return copied.reference
    }
    nonisolated private static func copy(_ source: URL, root: URL) throws -> NotesCopiedBlob {
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
        return NotesCopiedBlob(reference: .object(["digest": .string(digest), "size": .number(Double(size)), "state": .string("present")]), file: destination)
    }
    func stage(_ bytes: Data) async throws -> JSON {
        let root = self.root
        let job = Task.detached(priority: .utility) {
            let temporary = root.appendingPathComponent(UUID().uuidString)
            try bytes.write(to: temporary, options: .withoutOverwriting)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: temporary.path)
            defer { try? FileManager.default.removeItem(at: temporary) }
            return try Self.copy(temporary, root: root)
        }
        return try await finish(job)
    }
    func file(_ digest: String) -> URL? { files["notes-blob:" + digest] }
    func handles(_ references: [JSON]) -> [String: String] {
        var result: [String: String] = [:]
        for ref in references where files["notes-blob:" + ref["digest"].string] != nil { result[ref["digest"].string] = "notes-blob:" + ref["digest"].string }
        return result
    }
    func prune(_ references: [JSON]) throws {
        let active = Set(references.map { $0["digest"].string })
        let cutoff = Date().addingTimeInterval(-3600)
        for file in try FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: [.contentModificationDateKey, .isRegularFileKey, .isSymbolicLinkKey]) {
            guard !active.contains(file.lastPathComponent) else { continue }
            let info = try file.resourceValues(forKeys: [.contentModificationDateKey, .isRegularFileKey, .isSymbolicLinkKey])
            guard info.isRegularFile == true, info.isSymbolicLink != true,
                  let modified = info.contentModificationDate, modified < cutoff else { continue }
            try FileManager.default.removeItem(at: file)
            files.removeValue(forKey: "notes-blob:" + file.lastPathComponent)
        }
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
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(String(handle.dropFirst("notes-blob:".count))), "eof": .bool(UInt64(offset + bytes.count) == size)])
    }
}
