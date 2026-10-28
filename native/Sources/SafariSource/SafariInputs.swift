import Foundation
import Darwin

func safariInputs(_ directory: URL, kind: String) -> String {
    let filename = kind == "history" ? "History.db" : kind == "cloud" ? "CloudTabs.db" : "Bookmarks.plist"
    let path = directory.appendingPathComponent(filename).resolvingSymlinksInPath().path
    func stamp(_ path: String, wal: Bool) -> String {
        var value = stat()
        guard Darwin.stat(path, &value) == 0 else { return wal ? "no_frames" : "missing" }
        if wal && value.st_size <= 32 { return "no_frames" }
        return "\(value.st_dev):\(value.st_ino):\(value.st_birthtimespec.tv_sec):\(value.st_birthtimespec.tv_nsec):\(value.st_size):\(value.st_mtimespec.tv_sec):\(value.st_mtimespec.tv_nsec):\(value.st_mode):\(value.st_uid):\(value.st_gid)"
    }
    return stamp(path, wal: false) + (kind == "bookmarks" ? "" : "|" + stamp(path + "-wal", wal: true))
}
