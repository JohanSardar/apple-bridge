import Foundation
import SourceSupport

public enum CalendarPostcondition {
    public static func matches(actual: JSON, expected: JSON, allDay: Bool) -> Bool {
        guard case .object = actual, case .object = expected else { return false }
        for (field, requested) in expected.object {
            let stored = actual[field]
            if ["notes", "location", "url"].contains(field), (stored == .null || stored == .string("")), (requested == .null || requested == .string("")) { continue }
            if field == "recurrence" {
                guard unordered(stored.array, requested.array, equal: { rule($0, $1, allDay: allDay) }) else { return false }
            } else if field == "alarms" {
                guard unordered(stored.array, requested.array, equal: { $0 == $1 }) else { return false }
            } else if stored != requested { return false }
        }
        return true
    }

    private static func unordered(_ actual: [JSON], _ expected: [JSON], equal: (JSON, JSON) -> Bool) -> Bool {
        guard actual.count == expected.count else { return false }
        var remaining = actual
        for item in expected {
            guard let index = remaining.firstIndex(where: { equal($0, item) }) else { return false }
            remaining.remove(at: index)
        }
        return true
    }

    private static func rule(_ actual: JSON, _ expected: JSON, allDay: Bool) -> Bool {
        var a = actual.object, b = expected.object
        if actual["days_of_week"].array.isEmpty && expected["days_of_week"].array.isEmpty { a.removeValue(forKey: "first_day_of_week"); b.removeValue(forKey: "first_day_of_week") }
        if allDay {
            for fields in [true, false] {
                var value = fields ? a : b
                if let date = parseTimestamp(value["end"]?.string ?? "") { value["end"] = .string(CalendarTime.floating(date, zone: TimeZone(secondsFromGMT: 0)!)) }
                if fields { a = value } else { b = value }
            }
        }
        return a == b
    }

    public static func evidence(native: String, fingerprint: String, actual: JSON, expected: JSON, allDay: Bool) -> JSON {
        let observed = matches(actual: actual, expected: expected, allDay: allDay)
        var result: [String: JSON] = ["native_key": .string(native), "fingerprint": .string(fingerprint), "observed": .bool(observed), "native_result": actual, "native_expected": expected, "native_expected_all_day": .bool(allDay), "absent": .bool(false)]
        if !observed { result["disposition"] = .string("indeterminate"); result["reason"] = .string("native_postcondition_mismatch") }
        return .object(result)
    }
}
