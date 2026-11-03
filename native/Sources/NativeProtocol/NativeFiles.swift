import Foundation
import Darwin
import CryptoKit
import SourceSupport

public actor NativeFiles {
    private let root: URL
    public init(root: URL) throws {
        self.root = root.standardizedFileURL
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        guard root.standardizedFileURL == root.resolvingSymlinksInPath() else { throw SourceFailure.invalid }
        var info = stat()
        guard lstat(root.path, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o700, info.st_mode & S_IFMT == S_IFDIR else { throw SourceFailure.invalid }
    }

    public func write(_ bytes: Data, mime: String) throws -> JSON {
        guard bytes.count <= 64 * 1024 * 1024, mime.utf8.count <= 128 else { throw SourceFailure.invalid }
        let name = UUID().uuidString
        let file = root.appendingPathComponent(name)
        let descriptor = Darwin.open(file.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        do { try handle.write(contentsOf: bytes); try handle.synchronize(); try handle.close() }
        catch { try? FileManager.default.removeItem(at: file); throw error }
        return .object(["path": .string(name), "size": .integer(UInt64(bytes.count)), "sha256": .string(Self.hash(bytes)), "mime": .string(mime)])
    }

    public func read(_ reference: JSON) throws -> Data {
        let name = reference["path"].string
        guard !name.isEmpty, name != ".", name != "..", !name.contains("/"), !name.contains("\0") else { throw SourceFailure.invalid }
        let descriptor = Darwin.open(root.appendingPathComponent(name).path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard descriptor >= 0 else { throw SourceFailure.invalid }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? handle.close() }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == getuid(), info.st_mode & S_IFMT == S_IFREG, info.st_mode & 0o777 == 0o600,
              info.st_nlink == 1, info.st_size >= 0, info.st_size <= 64 * 1024 * 1024, UInt64(info.st_size) == reference["size"].unsigned else { throw SourceFailure.invalid }
        let bytes = try handle.readToEnd() ?? Data()
        guard UInt64(bytes.count) == reference["size"].unsigned, Self.hash(bytes) == reference["sha256"].string else { throw SourceFailure.corrupt }
        return bytes
    }

    public func release(_ reference: JSON) throws {
        _ = try read(reference)
        try FileManager.default.removeItem(at: root.appendingPathComponent(reference["path"].string))
    }

    private static func hash(_ bytes: Data) -> String { SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined() }
}
