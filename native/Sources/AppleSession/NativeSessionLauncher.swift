import Foundation
import Darwin
import SourceSupport

enum NativeSessionLauncher {
    struct Launched: Sendable {
        let descriptor: Int32
        let proxy: SessionProxy
        func stop() {
            shutdown(descriptor, SHUT_RDWR)
            proxy.reap()
            Darwin.close(descriptor)
        }
    }

    static func launch(socket path: String, secretFile: String, home: String) throws -> Launched {
        var info = stat()
        guard lstat(secretFile, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600, info.st_mode & S_IFMT == S_IFREG else { throw SourceFailure.invalid }
        let parent = URL(fileURLWithPath: path).deletingLastPathComponent().path
        guard lstat(parent, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o700, info.st_mode & S_IFMT == S_IFDIR else { throw SourceFailure.invalid }
        guard lstat(path, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600, info.st_mode & S_IFMT == S_IFSOCK else { throw SourceFailure.invalid }
        let gateway = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard gateway >= 0 else { throw SourceFailure.unavailable }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8) + [0]
        guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { Darwin.close(gateway); throw SourceFailure.invalid }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
        let connected = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(gateway, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        var uid: uid_t = 0
        var gid: gid_t = 0
        guard connected == 0, getpeereid(gateway, &uid, &gid) == 0, uid == getuid() else { Darwin.close(gateway); throw SourceFailure.unavailable }
        let binary = ProcessInfo.processInfo.environment["AB_SESSION_BINARY"] ?? Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/ab-session").path
        let proxy: SessionProxy
        do { proxy = try SessionProxy(gateway: gateway, binary: binary, home: home, effects: false, session: secretFile) }
        catch { Darwin.close(gateway); throw error }
        Darwin.close(gateway)
        Darwin.close(proxy.nativeDescriptor)
        return Launched(descriptor: proxy.descriptor, proxy: proxy)
    }
}
