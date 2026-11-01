import Foundation
@preconcurrency import EventKit
import SourceSupport

public enum CalendarEventKitFields {
    public static func identitySearchIntervals(around now: Date) -> [DateInterval] {
        let calendar = Calendar(identifier: .gregorian)
        return stride(from: -12, through: 0, by: 3).compactMap { years in
            guard let start = calendar.date(byAdding: .year, value: years, to: now),
                  let end = calendar.date(byAdding: .year, value: min(years + 3, 1), to: now) else { return nil }
            return DateInterval(start: start, end: end)
        }
    }

    public static func structuredLocation(_ location: EKStructuredLocation?) -> JSON {
        guard let location else { return .null }
        let coordinate = location.geoLocation?.coordinate
        return .object([
            "title": location.title.map(JSON.string) ?? .null,
            "address": .null,
            "latitude": coordinate.map { .number($0.latitude) } ?? .null,
            "longitude": coordinate.map { .number($0.longitude) } ?? .null,
            "radius": .number(location.radius)
        ])
    }

    public static func participants(_ attendees: [EKParticipant]?) -> JSON {
        .array((attendees ?? []).map { attendee in
            .object([
                "name": attendee.name.map(JSON.string) ?? .null,
                "url": .string(attendee.url.absoluteString),
                "status": .number(Double(attendee.participantStatus.rawValue)),
                "role": .number(Double(attendee.participantRole.rawValue)),
                "type": .number(Double(attendee.participantType.rawValue)),
                "is_current_user": .bool(attendee.isCurrentUser)
            ])
        })
    }

    public static func ownerParticipationStatus(_ attendees: [EKParticipant]?) -> JSON {
        let statuses = Set((attendees ?? []).filter(\.isCurrentUser).map { $0.participantStatus.rawValue })
        guard statuses.count == 1, let status = statuses.first else { return .null }
        return .number(Double(status))
    }
}
