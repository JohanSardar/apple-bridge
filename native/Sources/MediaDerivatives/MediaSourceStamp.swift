import Foundation
import Darwin

public struct MediaSourceStamp: Sendable, Equatable {
    let digest: String
    let device: Int32
    let inode: UInt64
    let size: Int64
    let modifiedSeconds: Int
    let modifiedNanos: Int
    public init(_ url: URL, digest: String) throws {
        var value = stat()
        guard Darwin.fstatat(AT_FDCWD, url.path, &value, 0) == 0 else { throw MediaSourceChanged() }
        self.digest = digest
        device = value.st_dev
        inode = value.st_ino
        size = value.st_size
        modifiedSeconds = value.st_mtimespec.tv_sec
        modifiedNanos = value.st_mtimespec.tv_nsec
    }
    public func validate(_ url: URL) throws {
        guard self == (try MediaSourceStamp(url, digest: digest)) else { throw MediaSourceChanged() }
    }
}

public struct MediaSourceChanged: Error { public init() {} }
