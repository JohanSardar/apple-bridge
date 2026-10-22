import Foundation
import Darwin

final class MediaLease: @unchecked Sendable {
    private let descriptor: Int32
    private init(_ descriptor: Int32) { self.descriptor = descriptor }
    static func acquire(_ directory: URL) async throws -> MediaLease {
        let descriptor = Darwin.open(directory.appendingPathComponent(".derive.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        do {
            while flock(descriptor, LOCK_EX | LOCK_NB) != 0 {
                guard errno == EWOULDBLOCK || errno == EAGAIN else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
                try await MediaWork.yield()
                try await Task.sleep(for: .milliseconds(20))
            }
            try Task.checkCancellation()
            return MediaLease(descriptor)
        } catch { Darwin.close(descriptor); throw error }
    }
    static func cleanIfIdle(_ directory: URL) {
        let descriptor = Darwin.open(directory.appendingPathComponent(".derive.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { return }
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { Darwin.close(descriptor); return }
        let lease = MediaLease(descriptor)
        defer { withExtendedLifetime(lease) {} }
        try? removeAbandonedExports(directory)
    }
    static func removeStageFiles(_ directory: URL) throws {
        for name in ["thumb.jpg", "thumb.png", "display.jpg", "display.png", "poster.jpg", "poster.png", "video.mp4", "live.mp4", "audio.m4a"] {
            let file = directory.appendingPathComponent(name)
            guard FileManager.default.fileExists(atPath: file.path) else { continue }
            let values = try file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
            if values.isRegularFile == true && values.isSymbolicLink != true { try FileManager.default.removeItem(at: file) }
        }
    }
    static func removeAbandonedExports(_ directory: URL) throws {
        for file in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey]) where ["mp4", "m4a", "mov"].contains(file.pathExtension) && UUID(uuidString: file.deletingPathExtension().lastPathComponent) != nil {
            let values = try file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
            if values.isRegularFile == true && values.isSymbolicLink != true { try FileManager.default.removeItem(at: file) }
        }
    }
    deinit { flock(descriptor, LOCK_UN); Darwin.close(descriptor) }
}
