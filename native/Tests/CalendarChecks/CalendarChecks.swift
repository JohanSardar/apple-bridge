import Foundation
import CalendarSource
import EventKit
import Darwin

@main
struct CalendarChecks {
    static func main() async throws {
        if CommandLine.arguments.contains("--store-reader") { try await CalendarStoreChecks.run(); return }
        if CommandLine.arguments.contains("--hint-coverage") {
            let checks = [
                "late_coalesced_hint_requires_followup": !CalendarSource.hintIsCovered(first: 100, latest: 150, scanStarted: 125),
                "all_hints_before_scan_are_skipped": CalendarSource.hintIsCovered(first: 100, latest: 120, scanStarted: 125),
                "hint_at_scan_start_requires_followup": !CalendarSource.hintIsCovered(first: 100, latest: 125, scanStarted: 125)
            ]
            let output = try JSONSerialization.data(withJSONObject: checks, options: [.sortedKeys])
            print(String(decoding: output, as: UTF8.self))
            exit(checks.values.allSatisfy { $0 } ? 0 : 1)
        }
        if CommandLine.arguments.contains("--invalidated-event") {
            let store = EKEventStore()
            let event = EKEvent(eventStore: store)
            event.title = "[ab-test invalidated-event]"
            event.startDate = Date(timeIntervalSince1970: 1_792_000_000)
            event.endDate = event.startDate.addingTimeInterval(3600)
            guard !CalendarSource.canMaterialize(event) else { print("invalidated calendar object accepted"); exit(1) }
            print("invalidated calendar object requires fresh rebuild")
            return
        }

        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/New_York")!
        let now = try CalendarTime.date("2026-03-08T16:00:00Z", allDay: false, zone: calendar.timeZone)
        let window = CalendarTime.window(now, calendar: calendar)
        let chunks = CalendarTime.chunks(now, calendar: calendar)
        precondition(chunks.count >= 5)
        precondition(calendar.dateComponents([.day], from: calendar.startOfDay(for: now), to: window.start).day == -730)
        precondition(calendar.dateComponents([.day], from: calendar.startOfDay(for: now), to: window.end).day == 730)
        precondition(calendar.dateComponents([.day], from: chunks[0].start, to: chunks[0].end).day == 60)
        let ordered = chunks.sorted { $0.start < $1.start }
        precondition(ordered.first!.start == window.start && ordered.last!.end == window.end)
        for index in ordered.indices {
            let chunk = ordered[index]
            precondition(chunk.end <= calendar.date(byAdding: .year, value: 1, to: chunk.start)!)
            if index > 0 { precondition(ordered[index - 1].end == chunk.start) }
        }
        let before = try CalendarTime.date("2026-03-07T14:00:00Z", allDay: false, zone: calendar.timeZone)
        let after = calendar.date(byAdding: .day, value: 1, to: before)!
        precondition(calendar.component(.hour, from: before) == 9 && calendar.component(.hour, from: after) == 9)
        precondition(after.timeIntervalSince(before) == 23 * 3600)
        let fall = try CalendarTime.date("2026-10-31T13:00:00Z", allDay: false, zone: calendar.timeZone)
        let next = calendar.date(byAdding: .day, value: 1, to: fall)!
        precondition(calendar.component(.hour, from: next) == 9 && next.timeIntervalSince(fall) == 25 * 3600)
        let floating = try CalendarTime.date("2026-03-08", allDay: true, zone: calendar.timeZone)
        precondition(CalendarTime.floating(floating, zone: calendar.timeZone) == "2026-03-08")
        let tokyo = TimeZone(identifier: "Asia/Tokyo")!
        let tokyoDate = try CalendarTime.date("2026-03-08", allDay: true, zone: tokyo)
        precondition(CalendarTime.floating(tokyoDate, zone: tokyo) == "2026-03-08")
        precondition(CalendarTime.identity("allday", occurrence: floating, floatingZone: calendar.timeZone) == "allday|2026-03-08")
        precondition(CalendarTime.identity("allday", occurrence: tokyoDate, floatingZone: tokyo) == "allday|2026-03-08")
        let springEnd = try CalendarTime.recurrenceEnd(start: before, frequency: "daily", interval: 1, count: 3, zone: calendar.timeZone)
        let expectedSpring = try CalendarTime.date("2026-03-09T13:00:00Z", allDay: false, zone: calendar.timeZone)
        precondition(springEnd == expectedSpring)
        let fallEnd = try CalendarTime.recurrenceEnd(start: fall, frequency: "daily", interval: 1, count: 3, zone: calendar.timeZone)
        let expectedFall = try CalendarTime.date("2026-11-02T14:00:00Z", allDay: false, zone: calendar.timeZone)
        precondition(fallEnd == expectedFall)
        for zone in [calendar.timeZone, tokyo] {
            let civilStart = try CalendarTime.date("2026-10-31", allDay: true, zone: zone)
            let civilEnd = try CalendarTime.recurrenceEnd(start: civilStart, frequency: "daily", interval: 2, count: 3, zone: zone)
            precondition(CalendarTime.floating(civilEnd, zone: zone) == "2026-11-04")
        }
        let monthStart = try CalendarTime.date("2026-01-31", allDay: true, zone: calendar.timeZone)
        let monthEnd = try CalendarTime.recurrenceEnd(start: monthStart, frequency: "monthly", interval: 1, count: 3, zone: calendar.timeZone)
        precondition(CalendarTime.floating(monthEnd, zone: calendar.timeZone) == "2026-05-31")
        let leapStart = try CalendarTime.date("2024-02-29", allDay: true, zone: calendar.timeZone)
        let leapEnd = try CalendarTime.recurrenceEnd(start: leapStart, frequency: "yearly", interval: 1, count: 2, zone: calendar.timeZone)
        precondition(CalendarTime.floating(leapEnd, zone: calendar.timeZone) == "2028-02-29")
        let gapStart = try CalendarTime.date("2027-03-13T07:30:00Z", allDay: false, zone: calendar.timeZone)
        let gapSecond = try CalendarTime.recurrenceEnd(start: gapStart, frequency: "daily", interval: 1, count: 2, zone: calendar.timeZone)
        let gapThird = try CalendarTime.recurrenceEnd(start: gapStart, frequency: "daily", interval: 1, count: 3, zone: calendar.timeZone)
        let expectedGapSecond = try CalendarTime.date("2027-03-14T07:30:00Z", allDay: false, zone: calendar.timeZone)
        precondition(gapSecond == expectedGapSecond)
        let expectedGapThird = try CalendarTime.date("2027-03-15T06:30:00Z", allDay: false, zone: calendar.timeZone)
        precondition(gapThird == expectedGapThird)
        let foldStart = try CalendarTime.date("2026-10-31T05:30:00Z", allDay: false, zone: calendar.timeZone)
        let foldSecond = try CalendarTime.recurrenceEnd(start: foldStart, frequency: "daily", interval: 1, count: 2, zone: calendar.timeZone)
        let expectedFoldSecond = try CalendarTime.date("2026-11-01T05:30:00Z", allDay: false, zone: calendar.timeZone)
        precondition(foldSecond == expectedFoldSecond)
        try CalendarRecurrenceChecks.run()
        let original = CalendarTime.identity("series", occurrence: before)
        precondition(original == "series|2026-03-07T14:00:00.000Z")
        let sameOriginal = try CalendarTime.date("2026-03-07T09:00:00-05:00", allDay: false, zone: calendar.timeZone)
        precondition(CalendarTime.identity("series", occurrence: sameOriginal) == original)
        precondition(original != CalendarTime.identity("series", occurrence: after))
        precondition(CalendarTime.identity("single", occurrence: nil) == "single")
        print("Calendar time checks passed: chunk coverage, leap/DST days, wall time, floating dates, recurrence end bounds, occurrence identity")
    }
}
