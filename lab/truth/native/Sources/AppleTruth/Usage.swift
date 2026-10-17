import Foundation
import CryptoKit

enum UsageTruthFailure: Error { case streamUnavailable, unknownDevice, metadataOpen, metadataQuery, metadataRead }

enum UsageTruth {
    private struct Event: Hashable { let at: Double; let start: Bool; let app: String }
    private static func digest(_ text: String) -> String { SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined() }
    static func observe(root path: String? = nil) throws -> [String: Any] {
        let biome = path.map { URL(fileURLWithPath: $0) } ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Biome")
        let root = biome.appendingPathComponent("streams/restricted/App.InFocus")
        let matcher = try NSRegularExpression(pattern: "\\x10.\\x18[\\x00\\x01]\\x21.{8}\\x32.", options: [.dotMatchesLineSeparators])
        let bundles = try NSRegularExpression(pattern: "^[A-Za-z0-9._-]+$")
        guard let dirs = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isRegularFileKey]) else { throw UsageTruthFailure.streamUnavailable }
        var grouped: [String: Set<Event>] = [:]
        for case let url as URL in dirs {
            if url.pathComponents.contains("tombstone") { dirs.skipDescendants(); continue }
            guard url.lastPathComponent.allSatisfy(\.isNumber), try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else { continue }
            let relative = String(url.path.dropFirst(root.path.count + 1)).split(separator: "/")
            guard let first = relative.first else { continue }
            let device = first == "local" ? "local" : (relative.count > 2 ? String(relative[1]) : "")
            guard !device.isEmpty else { continue }
            let data = [UInt8](try Data(contentsOf: url))
            let text = String(String.UnicodeScalarView(data.map { UnicodeScalar(Int($0))! })) as NSString
            for match in matcher.matches(in: text as String, range: NSRange(location: 0, length: text.length)) {
                let index = match.range.location, count = Int(data[index + 14]), begin = index + 15
                guard count > 0, begin + count <= data.count, let app = String(bytes: data[begin..<(begin + count)], encoding: .ascii), bundles.firstMatch(in: app, range: NSRange(location: 0, length: app.utf16.count)) != nil else { continue }
                let timeData = Data(data[(index + 5)..<(index + 13)])
                let time = timeData.withUnsafeBytes { Double(bitPattern: UInt64(littleEndian: $0.loadUnaligned(as: UInt64.self))) } + 978307200
                guard time > 1.6e9, time < 2.1e9 else { continue }
                grouped[device, default: []].insert(Event(at: time, start: data[index + 3] == 1, app: app))
            }
        }
        let formatter = DateFormatter(); formatter.dateFormat = "yyyy-MM-dd"; formatter.timeZone = .current; formatter.calendar = Calendar(identifier: .gregorian)
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = .current
        var rows: [String: [String: Double]] = [:]
        let locks = Set(["loginwindow", "com.apple.loginwindow", "com.apple.ScreenSaver.Engine"])
        let peers = try devicePeers(biome, immutable: path != nil)
        for (directory, set) in grouped {
            guard let peer = peers[directory] else { throw UsageTruthFailure.unknownDevice }
            let sorted = set.sorted { a, b in a.at == b.at ? (a.start == b.start ? a.app < b.app : !a.start) : a.at < b.at }
            var session: Event?, lock: Double?
            func finish(_ at: Double) {
                guard let session, at > session.at else { return }
                let end = min(at, session.at + 4 * 3600), unlockedEnd = min(end, lock ?? end)
                var position = session.at
                while position < end {
                    let date = Date(timeIntervalSince1970: position)
                    guard let next = calendar.dateInterval(of: .day, for: date)?.end.timeIntervalSince1970 else { return }
                    let stop = min(next, end), seconds = max(0, min(stop, unlockedEnd) - position)
                    let key = digest(peer.0 + "/" + formatter.string(from: date) + "@" + TimeZone.current.identifier + "/" + session.app)
                    rows[key, default: ["seconds_estimated": 0, "clipped_seconds": 0, "sessions": 0]]["seconds_estimated", default: 0] += seconds
                    rows[key, default: [:]]["clipped_seconds", default: 0] += stop - position - seconds
                    rows[key, default: [:]]["sessions", default: 0] += 1
                    position = stop
                }
            }
            for e in sorted {
                if peer.1 && locks.contains(e.app) { if e.start && session != nil && lock == nil { lock = e.at }; continue }
                if e.start { finish(e.at); session = e; lock = nil }
                else if session?.app == e.app { finish(e.at); session = nil; lock = nil }
            }
            if let lock { finish(lock) }
        }
        return ["status": "verified", "rows": rows, "device_count": grouped.count]
    }
}

import SQLite3
extension UsageTruth {
    private static func devicePeers(_ root: URL, immutable: Bool) throws -> [String: (String, Bool)] {
        var db: OpaquePointer?, query: OpaquePointer?
        let path = root.appendingPathComponent("sync/sync.db").path
        let input = immutable ? "file:" + path + "?immutable=1" : path
        guard sqlite3_open_v2(input, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_URI, nil) == SQLITE_OK else { if let db { sqlite3_close(db) }; throw UsageTruthFailure.metadataOpen }
        defer { sqlite3_close(db) }
        guard sqlite3_prepare_v2(db, "select device_identifier,me,platform from DevicePeer", -1, &query, nil) == SQLITE_OK else { throw UsageTruthFailure.metadataQuery }
        defer { sqlite3_finalize(query) }
        var result: [String: (String, Bool)] = [:]
        var step = sqlite3_step(query)
        while step == SQLITE_ROW {
            guard let pointer = sqlite3_column_text(query, 0) else { throw UsageTruthFailure.metadataRead }
            let id = String(cString: pointer)
            result[sqlite3_column_int(query, 1) == 1 ? "local" : id] = (id, sqlite3_column_int(query, 2) == 3)
            step = sqlite3_step(query)
        }
        guard step == SQLITE_DONE else { throw UsageTruthFailure.metadataRead }
        return result
    }
}
