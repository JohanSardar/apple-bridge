import Foundation
import Darwin
import CryptoKit
import SourceSupport

final class PeerWire: @unchecked Sendable {
    let descriptor: Int32
    private let writeLock = NSLock()
    private let closeLock = NSLock()
    private let blobWrites = DispatchQueue(label: "applebridge.ipc.blobs", qos: .utility)
    private var closed = false

    init(socket path: String, secretFile: String) throws {
        var info = stat()
        guard lstat(secretFile, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600, info.st_mode & S_IFMT == S_IFREG else { throw SourceFailure.invalid }
        let secret = try Data(contentsOf: URL(fileURLWithPath: secretFile))
        guard secret.count == 32 else { throw SourceFailure.invalid }
        let parent = URL(fileURLWithPath: path).deletingLastPathComponent().path
        guard lstat(parent, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o700, info.st_mode & S_IFMT == S_IFDIR else { throw SourceFailure.invalid }
        guard lstat(path, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600, info.st_mode & S_IFMT == S_IFSOCK else { throw SourceFailure.invalid }
        descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        do {
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
            let bytes = Array(path.utf8) + [0]
            guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { throw SourceFailure.invalid }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
            let connected = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
            }
            guard connected == 0 else { throw SourceFailure.unavailable }
            var uid: uid_t = 0
            var gid: gid_t = 0
            guard getpeereid(descriptor, &uid, &gid) == 0, uid == getuid() else { throw SourceFailure.invalid }
            var timeout = timeval(tv_sec: 15, tv_usec: 0)
            setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
            setsockopt(descriptor, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
            var enabled: Int32 = 1
            setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &enabled, socklen_t(MemoryLayout<Int32>.size))
            let challenge = try receive()
            guard challenge["t"].string == "Challenge", challenge["nonce"].string.count == 64 else { throw SourceFailure.invalid }
            let nonce = (0..<32).map { _ in UInt8.random(in: 0...255) }.map { String(format: "%02x", $0) }.joined()
            let a = challenge["nonce"].string
            func proof(_ role: String) -> String {
                HMAC<SHA256>.authenticationCode(for: Data("apple-bridge-ipc/1\0\(role)\0\(a)\0\(nonce)".utf8), using: SymmetricKey(data: secret)).map { String(format: "%02x", $0) }.joined()
            }
            try send(.object(["t": .string("Authenticate"), "nonce": .string(nonce), "proof": .string(proof("helper"))]))
            let authenticated = try receive()
            let expected = Array(proof("gateway").utf8)
            let actual = Array(authenticated["proof"].string.utf8)
            guard authenticated["t"].string == "Authenticated", actual.count == expected.count else { throw SourceFailure.invalid }
            var difference: UInt8 = 0
            for (left, right) in zip(actual, expected) { difference |= left ^ right }
            guard difference == 0 else { throw SourceFailure.invalid }
        } catch {
            Darwin.close(descriptor)
            throw error
        }
    }

    deinit { close() }

    func close() {
        closeLock.lock()
        guard !closed else { closeLock.unlock(); return }
        closed = true
        shutdown(descriptor, SHUT_RDWR)
        closeLock.unlock()
        writeLock.lock()
        Darwin.close(descriptor)
        writeLock.unlock()
    }

    private func read(_ size: Int) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: size)
        var position = 0
        while position < size {
            let count = bytes.withUnsafeMutableBytes { Darwin.recv(descriptor, $0.baseAddress!.advanced(by: position), size - position, 0) }
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw SourceFailure.unavailable }
            position += count
        }
        return Data(bytes)
    }

    func receive() throws -> JSON {
        let header = try read(4)
        let length = header.reduce(0) { ($0 << 8) | Int($1) }
        guard length > 0, length <= (1024 * 1024 + 64 * 1024) else { throw SourceFailure.invalid }
        let result = try JSONDecoder().decode(JSON.self, from: read(length))
        guard !result["t"].string.isEmpty, length <= 256 * 1024 || result["t"].string == "QueryResult" else { throw SourceFailure.invalid }
        return result
    }

    func send(_ value: JSON) throws {
        try write(Self.encode(value))
    }

    func sendBlob(_ value: JSON, trace: CaptureTrace, source: String) async throws {
        let queued = CaptureTrace.now()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            blobWrites.async { [self] in
                let started = CaptureTrace.now()
                do {
                    let bytes = try Self.encode(value)
                    let encoded = CaptureTrace.now()
                    try write(bytes)
                    trace.record("helper_blob_sent", fields: ["source": .string(source), "queued_ns": .integer(queued), "started_ns": .integer(started), "prepared_ns": .integer(encoded), "finished_ns": .integer(CaptureTrace.now())])
                    continuation.resume()
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    private func write(_ bytes: Data) throws {
        writeLock.lock()
        defer { writeLock.unlock() }
        closeLock.lock()
        let available = !closed
        closeLock.unlock()
        guard available else { throw SourceFailure.unavailable }
        try bytes.withUnsafeBytes { buffer in
            var position = 0
            while position < bytes.count {
                let count = Darwin.send(descriptor, buffer.baseAddress!.advanced(by: position), bytes.count - position, 0)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw SourceFailure.unavailable }
                position += count
            }
        }
    }

    static func encode(_ value: JSON) throws -> Data {
        let data = try JSONEncoder().encode(value)
        guard !value["t"].string.isEmpty, data.count <= (1024 * 1024 + 64 * 1024) else { throw SourceFailure.invalid }
        guard data.count <= 256 * 1024 || value["t"].string == "QueryResult" else { throw SourceFailure.invalid }
        let size = UInt32(data.count).bigEndian
        var bytes = withUnsafeBytes(of: size) { Data($0) }
        bytes.append(data)
        return bytes
    }
}
