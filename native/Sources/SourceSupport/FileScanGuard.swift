import Foundation

public struct FileScanGuard: Sendable {
    private var previous: [String: Set<String>] = [:]
    public init() {}
    public mutating func finish(root: String, keys: Set<String>, complete: Bool) -> Set<String> {
        guard complete else { return [] }
        let absent = CompleteScan.absence(previous: previous[root] ?? [], current: keys, complete: complete)
        previous[root] = keys
        return absent
    }
}

public extension CompleteScan {
    static func absence(previous: Set<String>, current: Set<String>, complete: Bool) -> Set<String> {
        complete ? previous.subtracting(current) : []
    }
}
