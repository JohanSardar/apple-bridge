import Foundation
import Darwin
import SourceSupport

public final class NativeWire: @unchecked Sendable {
    public static let maximumFrame = 256 * 1024
    private let descriptor: Int32
    private let writeLock = NSLock()

    public init(descriptor: Int32) throws {
        var kind: Int32 = 0
        var size = socklen_t(MemoryLayout<Int32>.size)
        var address = sockaddr_un()
        var addressSize = socklen_t(MemoryLayout<sockaddr_un>.size)
        guard getsockopt(descriptor, SOL_SOCKET, SO_TYPE, &kind, &size) == 0, kind == SOCK_STREAM,
              withUnsafeMutablePointer(to: &address, { pointer in
                  pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { getpeername(descriptor, $0, &addressSize) }
              }) == 0, address.sun_family == AF_UNIX else { throw SourceFailure.invalid }
        self.descriptor = descriptor
        var enabled: Int32 = 1
        guard setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &enabled, socklen_t(MemoryLayout<Int32>.size)) == 0 else { throw SourceFailure.unavailable }
    }

    public func receive() throws -> JSON {
        let header = try read(4)
        let length = header.reduce(0) { ($0 << 8) | Int($1) }
        guard length > 0, length <= Self.maximumFrame else { throw SourceFailure.invalid }
        let bytes = try read(length)
        let value = try JSONDecoder().decode(JSON.self, from: bytes)
        guard case .object = value else { throw SourceFailure.invalid }
        try Self.validateKeys(bytes)
        return value
    }

    private static func validateKeys(_ data: Data) throws {
        let bytes = Array(data)
        var stack: [Set<String>?] = []
        var position = 0
        while position < bytes.count {
            switch bytes[position] {
            case 123: stack.append([])
            case 91: stack.append(nil)
            case 125, 93: stack.removeLast()
            case 34:
                let start = position
                position += 1
                while position < bytes.count {
                    if bytes[position] == 92 { position += 2; continue }
                    if bytes[position] == 34 { break }
                    position += 1
                }
                let end = position + 1
                var next = end
                while next < bytes.count && [9, 10, 13, 32].contains(bytes[next]) { next += 1 }
                if next < bytes.count && bytes[next] == 58, let keys = stack.last ?? nil {
                    let key = try JSONDecoder().decode(String.self, from: Data(bytes[start..<end]))
                    guard !keys.contains(key) else { throw SourceFailure.invalid }
                    stack[stack.count - 1]?.insert(key)
                }
            default: break
            }
            position += 1
        }
    }

    public func send(_ value: JSON) throws {
        let body = try JSONEncoder().encode(value)
        guard !body.isEmpty, body.count <= Self.maximumFrame else { throw SourceFailure.invalid }
        var length = UInt32(body.count).bigEndian
        var bytes = withUnsafeBytes(of: &length) { Data($0) }
        bytes.append(body)
        writeLock.lock()
        defer { writeLock.unlock() }
        try bytes.withUnsafeBytes { buffer in
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.send(descriptor, buffer.baseAddress!.advanced(by: offset), bytes.count - offset, 0)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw SourceFailure.unavailable }
                offset += count
            }
        }
    }

    private func read(_ length: Int) throws -> Data {
        var bytes = Data(count: length)
        try bytes.withUnsafeMutableBytes { buffer in
            var offset = 0
            while offset < length {
                let count = recv(descriptor, buffer.baseAddress!.advanced(by: offset), length - offset, 0)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw SourceFailure.unavailable }
                offset += count
            }
        }
        return bytes
    }
}
