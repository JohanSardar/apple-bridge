import Foundation
import CryptoKit
import Darwin

public final class CaptureTrace: @unchecked Sendable {
    private let lock = NSLock()
    private let url: URL?
    public init(url: URL? = nil) { self.url = url }
    public static func now() -> UInt64 { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
    public func record(_ stage: String, keys: [String] = [], fields: [String: JSON] = [:]) {
        guard let url else { return }
        lock.lock()
        defer { lock.unlock() }
        var value = fields
        value["stage"] = .string(stage)
        value["uptime_ns"] = .integer(Self.now())
        value["wall_ns"] = .number(Date().timeIntervalSince1970 * 1_000_000_000)
        value["keys"] = .array(keys.map { .string(SHA256.hash(data: Data($0.utf8)).map { String(format: "%02x", $0) }.joined()) })
        guard var data = try? JSONEncoder().encode(JSON.object(value)) else { return }
        data.append(10)
        if !FileManager.default.fileExists(atPath: url.path) { _ = FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600]) }
        guard let file = try? FileHandle(forWritingTo: url) else { return }
        defer { try? file.close() }
        do { try file.seekToEnd(); try file.write(contentsOf: data) } catch { }
    }
}
