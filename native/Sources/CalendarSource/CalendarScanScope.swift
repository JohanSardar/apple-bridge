import Foundation
import SourceSupport

struct CalendarScanScope {
    var interval: DateInterval
    var calendars: Set<String>

    mutating func include(_ other: CalendarScanScope) {
        interval = DateInterval(start: min(interval.start, other.interval.start), end: max(interval.end, other.interval.end))
        calendars.formUnion(other.calendars)
    }

    func covers(_ observation: JSON) -> Bool {
        guard observation["collection"].string == "calendar.event" else { return false }
        let value = observation["data"]
        guard calendars.contains(value["calendar_id"]["$ref"]["native_key"].string) else { return false }
        let original = value["occurrence_date"].string.isEmpty ? value["start"].string : value["occurrence_date"].string
        guard let date = try? CalendarTime.date(original, allDay: value["all_day"].bool, zone: .current) else { return false }
        return date >= interval.start && date < interval.end
    }
}
