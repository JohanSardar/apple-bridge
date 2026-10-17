import Foundation
import Decoders
import SourceSupport

public enum UsageRollup {
    public static let locks: Set<String> = ["loginwindow", "com.apple.loginwindow", "com.apple.ScreenSaver.Engine", "bridge.power.sleep"]
    public static func records(_ input: [InFocusEvent], device: String, mac: Bool, zone: TimeZone = .current) -> [String: JSON] {
        let events = Set(input).sorted { a, b in
            if a.time != b.time { return a.time < b.time }
            if a.starting != b.starting { return !a.starting }
            return a.bundle < b.bundle
        }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = zone
        let formatter = DateFormatter(); formatter.calendar = calendar; formatter.timeZone = zone; formatter.dateFormat = "yyyy-MM-dd"
        var current: InFocusEvent?, lockedAt: Double?
        var result: [String: JSON] = [:]
        func close(_ end: Double) {
            guard let current, end > current.time else { return }
            let capped = min(end, current.time + 14400), credited = min(capped, lockedAt ?? capped)
            var cursor = current.time
            while cursor < capped {
                let date = Date(timeIntervalSince1970: cursor)
                guard let next = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: date)) else { return }
                let until = min(capped, next.timeIntervalSince1970)
                let day = formatter.string(from: date) + "@" + zone.identifier
                let key = device + "/" + day + "/" + current.bundle
                var value = result[key]?.object ?? ["device_id": .object(["$ref": .object(["collection": .string("usage.device"), "native_key": .string(device)])]), "day": .string(day), "bundle_id": .string(current.bundle), "provenance": .string("biome.app_infocus"), "decoder_version": .string(InFocus.version), "uncertainty": .string("focus estimates; settings_parity: unproven; unclosed sessions omitted; remote batches may lag")]
                let seconds = max(0, min(until, credited) - cursor)
                value["seconds_estimated"] = .number((value["seconds_estimated"]?.number ?? 0) + seconds)
                value["clipped_seconds"] = .number((value["clipped_seconds"]?.number ?? 0) + until - cursor - seconds)
                value["sessions"] = .number((value["sessions"]?.number ?? 0) + 1)
                value["latest_event_at"] = .string(timestamp(Date(timeIntervalSince1970: end)))
                result[key] = .object(value)
                cursor = until
            }
        }
        for event in events {
            if mac && locks.contains(event.bundle) {
                if event.starting, current != nil, lockedAt == nil { lockedAt = event.time }
                continue
            }
            if event.starting { close(event.time); current = event; lockedAt = nil }
            else if event.bundle == current?.bundle { close(event.time); current = nil; lockedAt = nil }
        }
        if let lockedAt { close(lockedAt) }
        return result
    }
}
