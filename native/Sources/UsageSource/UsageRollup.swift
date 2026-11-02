import Foundation
import Decoders
import SourceSupport

public enum UsageRollup {
    public static let locks: Set<String> = ["loginwindow", "com.apple.loginwindow", "com.apple.ScreenSaver.Engine", "bridge.power.sleep"]
    public static let excluded: Set<String> = ["com.apple.universalcontrol", "com.apple.UserNotificationCenter", "com.apple.systempreferences", "com.apple.controlcenter", "com.apple.notificationcenterui"]
    public static func records(_ input: [InFocusEvent], device: String, mac: Bool, zone: TimeZone = .current, blocked: [DateInterval] = []) -> [String: JSON] {
        let events = Set(input).sorted { a, b in
            if a.time != b.time { return a.time < b.time }
            if a.starting != b.starting { return !a.starting }
            return a.bundle < b.bundle
        }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = zone
        let formatter = DateFormatter(); formatter.calendar = calendar; formatter.timeZone = zone; formatter.dateFormat = "yyyy-MM-dd"
        var current: InFocusEvent?, lockedAt: Double?
        var daily: [String: Double] = [:]
        let barriers = blocked.sorted { $0.start < $1.start }
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
                var value = result[key]?.object ?? ["device_id": .object(["$ref": .object(["collection": .string("usage.device"), "native_key": .string(device)])]), "day": .string(day), "bundle_id": .string(current.bundle), "provenance": .string("biome.app_infocus"), "decoder_version": .string(InFocus.version), "uncertainty": .string("focus estimates, not active interaction; settings_parity: unproven; unclosed sessions omitted; four-hour cap; system UI excluded; historical lock/sleep gaps; remote batches may lag")]
                let upper = min(until, credited)
                var available = max(0, upper - cursor)
                var coveredUntil = cursor
                for interval in barriers where interval.start.timeIntervalSince1970 < upper && interval.end.timeIntervalSince1970 > cursor {
                    let lower = max(cursor, coveredUntil, interval.start.timeIntervalSince1970)
                    let end = min(upper, interval.end.timeIntervalSince1970)
                    available -= max(0, end - lower)
                    coveredUntil = max(coveredUntil, end)
                }
                if excluded.contains(current.bundle) { available = 0 }
                let seconds = min(max(0, available), max(0, 86400 - (daily[day] ?? 0)))
                daily[day, default: 0] += seconds
                value["usage_class"] = .string(excluded.contains(current.bundle) ? "excluded_system_ui" : "foreground_estimate")
                value["clipping_provenance"] = .string(blocked.isEmpty ? "focus_lock_and_live_power_only" : "focus_lock_live_power_and_local_knowledgeC_display_lock")
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
