import Foundation
import SourceSupport

enum CalendarParity {
    static let coverage: JSON = .array([
        limited("calendar.respond_invitation", reason: "calendar_accessibility_invitation_response_not_proven_on_this_os_build"),
        limited("calendar.set_travel_time", reason: "calendar_accessibility_travel_time_write_and_positive_units_not_proven_on_this_os_build"),
        limited("calendar.add_attachment", reason: "calendar_accessibility_attachment_add_not_proven_on_this_os_build"),
        limited("calendar.remove_attachment", reason: "calendar_accessibility_attachment_remove_not_proven_on_this_os_build")
    ])

    private static func limited(_ operation: String, reason: String) -> JSON {
        .object(["operation": .string(operation), "proven": .bool(false), "reason": .string(reason)])
    }
}
