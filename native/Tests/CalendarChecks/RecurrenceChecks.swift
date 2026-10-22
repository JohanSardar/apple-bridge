import Foundation
import SourceSupport
import CalendarSource

struct CalendarRecurrenceChecks {
    static func run() throws {
        let rule: JSON = .object(["frequency": .string("daily"), "interval": .number(2), "count": .number(2)])
        let expected: JSON = .object(["title": .string("Bridge Test"), "recurrence": .array([rule])])
        var wrong = rule.object
        wrong["count"] = .number(1)
        let actual: JSON = .object(["title": .string("Bridge Test"), "recurrence": .array([.object(wrong)])])
        let mismatch = CalendarPostcondition.evidence(native: "fixture", fingerprint: "fixture", actual: actual, expected: expected, allDay: false)
        precondition(mismatch["disposition"].string == "indeterminate" && !mismatch["observed"].bool)
        precondition(mismatch["native_result"] == actual && mismatch["reason"].string == "native_postcondition_mismatch")
        precondition(CalendarPostcondition.matches(actual: expected, expected: expected, allDay: false))
        let clear: JSON = .object(["notes": .string("")])
        precondition(CalendarPostcondition.matches(actual: .object(["notes": .null]), expected: clear, allDay: false))
        precondition(!CalendarPostcondition.matches(actual: .object(["notes": .number(7)]), expected: clear, allDay: false))
        let a: JSON = .object(["recurrence": .array([.object(["frequency": .string("daily"), "interval": .number(1), "count": .number(0), "end": .string("2026-11-04T00:00:00Z")])])])
        let b: JSON = .object(["recurrence": .array([.object(["frequency": .string("daily"), "interval": .number(1), "count": .number(0), "end": .string("2026-11-04T05:00:00Z")])])])
        precondition(CalendarPostcondition.matches(actual: a, expected: b, allDay: true))
        precondition(!CalendarPostcondition.matches(actual: a, expected: b, allDay: false))
        let zone = TimeZone(identifier: "America/New_York")!
        let start = try CalendarTime.date("2026-11-01T06:30:00Z", allDay: false, zone: zone)
        let bound = try CalendarTime.recurrenceEnd(start: start, frequency: "daily", interval: 1, count: 2, zone: zone)
        let expectedBound = try CalendarTime.date("2026-11-02T06:30:00Z", allDay: false, zone: zone)
        precondition(bound == expectedBound)
        let twoDayDuration = start.addingTimeInterval(2 * 86400)
        precondition(bound < twoDayDuration)
        let floating = try CalendarTime.date("2026-11-01", allDay: true, zone: TimeZone(secondsFromGMT: 0)!)
        let floatingBound = try CalendarTime.recurrenceEnd(start: floating, frequency: "daily", interval: 1, count: 2, zone: TimeZone(secondsFromGMT: 0)!)
        precondition(CalendarTime.floating(floatingBound, zone: TimeZone(secondsFromGMT: 0)!) == "2026-11-02")
        var invalid = false
        do { _ = try CalendarTime.recurrenceEnd(start: start, frequency: "weekly", interval: Int.max, count: 2, zone: zone) } catch { invalid = true }
        precondition(invalid)
    }
}
