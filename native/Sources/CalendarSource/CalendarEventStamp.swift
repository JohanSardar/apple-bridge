import Foundation
@preconcurrency import EventKit
import AgendaGate
import SourceSupport

enum CalendarStampField { case other, notes, attendees, alarms }

struct CalendarStampProfile {
    var nativeNS: UInt64 = 0
    var hashNS: UInt64 = 0
    var notesNS: UInt64 = 0
    var attendeesNS: UInt64 = 0
    var alarmsNS: UInt64 = 0
}

struct CalendarOccurrenceKey: Hashable {
    let identifier: String
    let occurrence: Date?

    @AgendaActor
    init(_ event: EKEvent) {
        identifier = event.calendarItemIdentifier
        occurrence = event.hasRecurrenceRules || event.isDetached ? event.occurrenceDate : nil
    }
}

struct CalendarMaterialization {
    let stamp: Int
    let identity: String
}

struct CalendarSeriesStamp {
    let modified: Date?
    let recurring: Bool
    let fingerprint: Int

    @AgendaActor
    init(_ event: EKEvent, master: EKEvent?, floatingZone: String, profile: inout CalendarStampProfile?) {
        var hash = Hasher()
        @AgendaActor
        func read<T>(_ value: @autoclosure @AgendaActor () -> T, field: CalendarStampField = .other) -> T {
            guard profile != nil else { return value() }
            let started = CaptureTrace.now()
            let result = value()
            let elapsed = CaptureTrace.now() - started
            profile?.nativeNS += elapsed
            switch field {
            case .notes: profile?.notesNS += elapsed
            case .attendees: profile?.attendeesNS += elapsed
            case .alarms: profile?.alarmsNS += elapsed
            case .other: break
            }
            return result
        }
        @AgendaActor
        func combine<T: Hashable>(_ value: @autoclosure @AgendaActor () -> T, field: CalendarStampField = .other) {
            let result = read(value(), field: field)
            guard profile != nil else { hash.combine(result); return }
            let started = CaptureTrace.now()
            hash.combine(result)
            profile?.hashNS += CaptureTrace.now() - started
        }
        let isRecurring = read(event.hasRecurrenceRules)
        let lastModified = isRecurring ? read(master?.lastModifiedDate) : read(event.lastModifiedDate)
        recurring = isRecurring
        modified = lastModified
        combine(lastModified)
        combine(isRecurring)
        combine(event.calendar.calendarIdentifier)
        combine(event.calendar.source.sourceIdentifier)
        combine(floatingZone)
        fingerprint = hash.finalize()
    }
}

struct CalendarEventStamp {
    @AgendaActor
    static func fingerprint(_ event: EKEvent, series: CalendarSeriesStamp, now: Date, profile: inout CalendarStampProfile?) -> Int? {
        var hash = Hasher()
        @AgendaActor
        func read<T>(_ value: @autoclosure @AgendaActor () -> T, field: CalendarStampField = .other) -> T {
            guard profile != nil else { return value() }
            let started = CaptureTrace.now()
            let result = value()
            let elapsed = CaptureTrace.now() - started
            profile?.nativeNS += elapsed
            switch field {
            case .notes: profile?.notesNS += elapsed
            case .attendees: profile?.attendeesNS += elapsed
            case .alarms: profile?.alarmsNS += elapsed
            case .other: break
            }
            return result
        }
        @AgendaActor
        func combine<T: Hashable>(_ value: @autoclosure @AgendaActor () -> T, field: CalendarStampField = .other) {
            let result = read(value(), field: field)
            guard profile != nil else { hash.combine(result); return }
            let started = CaptureTrace.now()
            hash.combine(result)
            profile?.hashNS += CaptureTrace.now() - started
        }
        guard !read(event.isDetached), series.recurring == read(event.hasRecurrenceRules), let modified = read(event.lastModifiedDate),
              let masterModified = series.modified, modified.timeIntervalSince(now) < -2,
              masterModified.timeIntervalSince(now) < -2 else { return nil }
        combine(series.fingerprint)
        combine(event.calendarItemIdentifier)
        combine(event.calendar.calendarIdentifier)
        combine(event.calendar.source.sourceIdentifier)
        combine(event.eventIdentifier)
        combine(event.calendarItemExternalIdentifier)
        combine(modified)
        combine(event.occurrenceDate)
        combine(event.location)
        let alarms = read(event.alarms, field: .alarms)
        combine(alarms?.count)
        for alarm in alarms ?? [] { combine(alarm.absoluteDate, field: .alarms); combine(alarm.relativeOffset, field: .alarms) }
        return hash.finalize()
    }
}
