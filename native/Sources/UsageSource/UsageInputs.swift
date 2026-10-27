import Foundation

public struct UsageInputs: Equatable, Sendable {
    let files: [String]
    let power: Int
    let zone: String
    public init?(_ root: URL, power: Int, zone: TimeZone = .current) {
        let keys: [URLResourceKey] = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey]
        func entry(_ file: URL) -> String? {
            guard let values = try? file.resourceValues(forKeys: Set(keys)) else { return nil }
            return file.path + ":" + String(values.fileSize ?? -1) + ":" + String(values.contentModificationDate?.timeIntervalSinceReferenceDate.bitPattern ?? 0)
        }
        var failure = false
        let streams = root.appendingPathComponent("streams/restricted/App.InFocus")
        guard let iterator = FileManager.default.enumerator(at: streams, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles], errorHandler: { _, _ in failure = true; return false }) else { return nil }
        var files: [String] = []
        for case let file as URL in iterator where (try? file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true {
            guard let value = entry(file) else { return nil }
            files.append(value)
        }
        for name in ["sync/sync.db", "sync/sync.db-wal"] {
            let file = root.appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: file.path) { guard let value = entry(file) else { return nil }; files.append(value) }
        }
        guard !failure else { return nil }
        self.files = files.sorted()
        self.power = power
        self.zone = zone.identifier
    }
}
