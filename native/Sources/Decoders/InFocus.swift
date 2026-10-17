import Foundation

public struct InFocusEvent: Hashable, Sendable {
    public let time: Double
    public let starting: Bool
    public let bundle: String
    public init(time: Double, starting: Bool, bundle: String) {
        self.time = time; self.starting = starting; self.bundle = bundle
    }
}

public enum InFocus {
    public static let version = "infocus-segb-1"
    public static func decode(_ data: Data) -> [InFocusEvent] {
        let bytes = [UInt8](data)
        guard bytes.count >= 16 else { return [] }
        var events: [InFocusEvent] = []
        for index in 0...(bytes.count - 16) {
            guard bytes[index] == 0x10, bytes[index + 2] == 0x18, bytes[index + 3] <= 1, bytes[index + 4] == 0x21, bytes[index + 13] == 0x32 else { continue }
            let length = Int(bytes[index + 14]), start = index + 15
            guard length > 0, start + length <= bytes.count else { continue }
            let bundle = bytes[start..<(start + length)]
            guard bundle.allSatisfy({ (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || [45, 46, 95].contains($0) }) else { continue }
            var bits: UInt64 = 0
            for offset in 0..<8 { bits |= UInt64(bytes[index + 5 + offset]) << (offset * 8) }
            let time = Double(bitPattern: bits) + 978307200
            guard time.isFinite, time > 1.6e9, time < 2.1e9 else { continue }
            events.append(InFocusEvent(time: time, starting: bytes[index + 3] == 1, bundle: String(decoding: bundle, as: UTF8.self)))
        }
        return events
    }
}
