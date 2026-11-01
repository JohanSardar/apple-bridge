import Foundation
@preconcurrency import EventKit
import SourceSupport

public enum CalendarEventKitFields {
    public static func invitationAddress(_ url: URL) -> String? {
        guard url.scheme?.lowercased() == "mailto", url.query == nil, url.fragment == nil,
              let address = String(url.absoluteString.dropFirst(7)).removingPercentEncoding?.lowercased() else { return nil }
        let parts = address.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty,
              parts[0].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || ".-_+".contains($0)) }),
              parts[1].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || ".-".contains($0)) }) else { return nil }
        return address
    }

    public static func selfInvitationOrganizer(from verifiedAddresses: Set<String>, token: String) -> String? {
        guard !token.isEmpty, token.count <= 20,
              token.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }) else { return nil }
        for address in verifiedAddresses.sorted() {
            let parts = address.lowercased().split(separator: "@", omittingEmptySubsequences: false)
            guard parts.count == 2, parts[1] == "gmail.com" else { continue }
            guard let local = parts[0].split(separator: "+", maxSplits: 1, omittingEmptySubsequences: false).first,
                  !local.isEmpty, local.count <= 30,
                  local.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == ".") }) else { continue }
            return String(local) + "+ab-test-" + token + "@gmail.com"
        }
        return nil
    }

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
