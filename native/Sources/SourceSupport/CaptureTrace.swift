import Foundation
import CryptoKit
import Darwin

public final class CaptureTrace: @unchecked Sendable {
    private let lock = NSLock()
    private let url: URL?
    private var output: FileHandle?
    deinit { try? output?.close() }
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
        if output == nil {
            let descriptor = Darwin.open(url.path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0o600)
            guard descriptor >= 0 else { return }
            output = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        }
        do { try output?.write(contentsOf: data) } catch { try? output?.close(); output = nil }
    }
}
