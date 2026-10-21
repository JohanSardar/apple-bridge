import Foundation
import SourceSupport

public struct StoreRefreshHistogram: Sendable {
    private var first: Double?
    private var warm: [Double] = []
    private var total = 0

    public init() {}

    public mutating func record(_ milliseconds: Double) -> JSON {
        total += 1
        if first == nil { first = milliseconds }
        else {
            warm.append(milliseconds)
            if warm.count > 4096 { warm.removeFirst() }
        }
        let sorted = warm.sorted()
        func percentile(_ fraction: Double) -> JSON {
            guard !sorted.isEmpty else { return .null }
            return .number(sorted[max(0, Int((Double(sorted.count) * fraction).rounded(.up)) - 1)])
        }
        return .object([
            "total_samples": .number(Double(total)), "first_ms": first.map(JSON.number) ?? .null,
            "warm": .object(["samples": .number(Double(sorted.count)), "p50_ms": percentile(0.5), "p95_ms": percentile(0.95), "max_ms": sorted.last.map(JSON.number) ?? .null])
        ])
    }
}
