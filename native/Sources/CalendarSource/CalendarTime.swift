import Foundation
import SourceSupport

public enum CalendarTime {
    public static func window(_ now: Date, calendar: Calendar = .current) -> DateInterval {
        let today = calendar.startOfDay(for: now)
        return DateInterval(start: calendar.date(byAdding: .day, value: -730, to: today)!, end: calendar.date(byAdding: .day, value: 730, to: today)!)
    }

    public static func chunks(_ now: Date, calendar: Calendar = .current) -> [DateInterval] {
        let bounds = window(now, calendar: calendar)
        let today = calendar.startOfDay(for: now)
        let near = DateInterval(start: calendar.date(byAdding: .day, value: -30, to: today)!, end: calendar.date(byAdding: .day, value: 30, to: today)!)
        var result = [near]
        for segment in [DateInterval(start: bounds.start, end: near.start), DateInterval(start: near.end, end: bounds.end)] {
            var start = segment.start
            while start < segment.end {
                let end = min(calendar.date(byAdding: .year, value: 1, to: start)!, segment.end)
                result.append(DateInterval(start: start, end: end))
                start = end
            }
        }
        return result
    }

    public static func floating(_ date: Date, zone: TimeZone) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let value = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", value.year!, value.month!, value.day!)
    }

    public static func date(_ value: String, allDay: Bool, zone: TimeZone) throws -> Date {
        if !allDay, let date = parseTimestamp(value) { return date }
        guard allDay, value.count == 10 else { throw SourceFailure.invalid }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = zone
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.isLenient = false
        guard let date = formatter.date(from: value), formatter.string(from: date) == value else { throw SourceFailure.invalid }
        return date
    }

    public static func identity(_ identifier: String, occurrence: Date?, floatingZone: TimeZone? = nil) -> String {
        identifier + (occurrence.map { date in "|" + (floatingZone.map { zone in floating(date, zone: zone) } ?? timestamp(date)) } ?? "")
    }
}
