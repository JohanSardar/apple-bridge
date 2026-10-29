import Foundation
import Darwin

public struct UsageInputs: Equatable, Sendable {
    let files: [String]
    private let stamps: [String: String]
    let power: Int
    let zone: String
    public init?(_ root: URL, power: Int, zone: TimeZone = .current, previous: UsageInputs? = nil) {
        func stamp(_ path: String) -> String {
            var value = stat()
            guard stat(path, &value) == 0 else { return "missing" }
            return "\(value.st_dev):\(value.st_ino):\(value.st_size):\(value.st_mtimespec.tv_sec):\(value.st_mtimespec.tv_nsec):\(value.st_ctimespec.tv_sec):\(value.st_ctimespec.tv_nsec)"
        }
        if let previous, previous.stamps[root.appendingPathComponent("streams/restricted/App.InFocus").path] != nil, previous.stamps.allSatisfy({ stamp($0.key) == $0.value }) {
            self.files = previous.files
            self.stamps = previous.stamps
            self.power = power
            self.zone = zone.identifier
            return
        }
        var stamps: [String: String] = [:]
        let keys: [URLResourceKey] = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey]
        let requested = Set(keys)
        let regular: Set<URLResourceKey> = [.isRegularFileKey]
        func entry(_ file: URL) -> String? {
            guard let values = try? file.resourceValues(forKeys: requested) else { return nil }
            return file.path + ":" + String(values.fileSize ?? -1) + ":" + String(values.contentModificationDate?.timeIntervalSinceReferenceDate.bitPattern ?? 0)
        }
        var failure = false
        let streams = root.appendingPathComponent("streams/restricted/App.InFocus")
        guard let iterator = FileManager.default.enumerator(at: streams, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles], errorHandler: { _, _ in failure = true; return false }) else { return nil }
        var files: [String] = []
        stamps[streams.path] = stamp(streams.path)
        for case let file as URL in iterator {
            stamps[file.path] = stamp(file.path)
            guard (try? file.resourceValues(forKeys: regular).isRegularFile) == true else { continue }
            guard let value = entry(file) else { return nil }
            files.append(value)
        }
        for name in ["sync/sync.db", "sync/sync.db-wal"] {
            let file = root.appendingPathComponent(name)
            stamps[file.path] = stamp(file.path)
            if FileManager.default.fileExists(atPath: file.path) { guard let value = entry(file) else { return nil }; files.append(value) }
        }
        guard !failure else { return nil }
        self.files = files.sorted()
        self.stamps = stamps
        self.power = power
        self.zone = zone.identifier
    }
}
