import Foundation

public struct FocusEvent: Hashable, Sendable {
    public let at: Double
    public let starting: Bool
    public let bundle: String
    public init(at: Double, starting: Bool, bundle: String) { self.at = at; self.starting = starting; self.bundle = bundle }
}

public enum InFocus {
    public static let version = "infocus-1"
    public static func decode(_ input: Data) -> [FocusEvent] {
        let bytes = Array(input)
        var events: [FocusEvent] = [], index = 0
        while index + 15 <= bytes.count {
            defer { index += 1 }
            guard bytes[index] == 16, bytes[index + 2] == 24, bytes[index + 3] <= 1, bytes[index + 4] == 33, bytes[index + 13] == 50 else { continue }
            var bits: UInt64 = 0
            for byte in 0..<8 { bits |= UInt64(bytes[index + 5 + byte]) << (8 * byte) }
            let at = Double(bitPattern: bits) + 978307200
            guard at.isFinite, at > 1.6e9, at < 2.1e9 else { continue }
            var cursor = index + 14, length = 0, shift = 0
            while cursor < bytes.count && shift < 28 {
                let byte = bytes[cursor]; cursor += 1
                length |= Int(byte & 127) << shift
                if byte < 128 { break }
                shift += 7
            }
            guard length > 0, length <= 1024, length <= bytes.count - cursor else { continue }
            let bundleBytes = bytes[cursor..<cursor + length]
            guard bundleBytes.allSatisfy({ (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || [45,46,95].contains($0) }), let bundle = String(bytes: bundleBytes, encoding: .utf8) else { continue }
            events.append(FocusEvent(at: at, starting: bytes[index + 3] == 1, bundle: bundle))
            index = cursor + length - 1
        }
        return events
    }
}

public struct AppDay: Sendable {
    public let day: String
    public let bundle: String
    public var seconds: Double
    public var clipped: Double
    public var sessions: Int
    public var latest: Double
}

public enum FocusUsage {
    public static func aggregate(_ input: [FocusEvent], mac: Bool, timeZone: TimeZone = .current) -> [AppDay] {
        let events = Set(input).sorted {
            if $0.at != $1.at { return $0.at < $1.at }
            if $0.starting != $1.starting { return !$0.starting }
            return $0.bundle < $1.bundle
        }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = timeZone
        var result: [String: AppDay] = [:]
        var current: FocusEvent?, previousApp: String?
        func record(_ start: Double, _ finish: Double, _ bundle: String, _ locked: Bool, _ latest: Double) {
            var cursor = start
            let end = min(finish, start + 14400)
            while cursor < end {
                let date = Date(timeIntervalSince1970: cursor)
                let parts = calendar.dateComponents([.year, .month, .day], from: date)
                let day = String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
                guard let next = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: date)) else { return }
                let through = min(end, next.timeIntervalSince1970)
                guard through > cursor else { return }
                let key = day + "/" + bundle
                var value = result[key] ?? AppDay(day: day, bundle: bundle, seconds: 0, clipped: 0, sessions: 0, latest: latest)
                if locked { value.clipped += through - cursor }
                else { value.seconds += through - cursor; value.sessions += 1 }
                value.latest = max(value.latest, latest)
                result[key] = value
                cursor = through
            }
        }
        func close(_ at: Double) {
            guard let current, at > current.at else { return }
            let locked = mac && (current.bundle == "loginwindow" || current.bundle == "com.apple.loginwindow" || current.bundle == "com.apple.ScreenSaver.Engine")
            if locked { if let previousApp { record(current.at, at, previousApp, true, at) } }
            else { record(current.at, at, current.bundle, false, at); previousApp = current.bundle }
        }
        for event in events {
            if event.starting { close(event.at); current = event }
            else if current?.bundle == event.bundle { close(event.at); current = nil }
        }
        return result.keys.sorted().compactMap { result[$0] }
    }
}
