import Foundation
import CalendarSource
import EventKit
import Darwin

@main
struct CalendarChecks {
    static func main() async throws {
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
        let original = CalendarTime.identity("series", occurrence: before)
        precondition(original == "series|2026-03-07T14:00:00.000Z")
        let sameOriginal = try CalendarTime.date("2026-03-07T09:00:00-05:00", allDay: false, zone: calendar.timeZone)
        precondition(CalendarTime.identity("series", occurrence: sameOriginal) == original)
        precondition(original != CalendarTime.identity("series", occurrence: after))
        precondition(CalendarTime.identity("single", occurrence: nil) == "single")
        print("Calendar time checks passed: chunk coverage, leap/DST days, wall time, floating dates, occurrence identity")
    }
}
