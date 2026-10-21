import Foundation
@preconcurrency import EventKit
import AgendaGate

struct CalendarEventStamp: Equatable {
    let identifier: String
    let eventIdentifier: String?
    let modified: Date
    let masterModified: Date
    let occurrence: Date?
    let start: Date
    let end: Date
    let calendar: String
    let account: String
    let external: String?
    let allDay: Bool
    let zone: String?
    let floatingZone: String

    @AgendaActor
    init?(_ event: EKEvent, masterModified: Date?, now: Date) {
        guard !event.isDetached, let modified = event.lastModifiedDate,
              let masterModified, modified.timeIntervalSince(now) < -2,
              masterModified.timeIntervalSince(now) < -2 else { return nil }
        identifier = event.calendarItemIdentifier
        eventIdentifier = event.eventIdentifier
        self.modified = modified
        self.masterModified = masterModified
        occurrence = event.occurrenceDate
        start = event.startDate
        end = event.endDate
        calendar = event.calendar.calendarIdentifier
        account = event.calendar.source.sourceIdentifier
        external = event.calendarItemExternalIdentifier
        allDay = event.isAllDay
        zone = event.timeZone?.identifier
        floatingZone = TimeZone.current.identifier
    }
}
