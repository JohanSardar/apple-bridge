import Foundation
import SourceSupport

enum RemindersParity {
    static let coverage: JSON = .array([
        proof("reminders.set_flag"),
        proof("reminders.indent"),
        proof("reminders.outdent"),
        proof("reminders.create_section"),
        proof("reminders.move_to_section"),
        proof("reminders.set_tags"),
        proof("reminders.set_list_emblem"),
        proof("reminders.set_list_group"),
        proof("reminders.reorder"),
        proof("reminders.add_image"),
        proof("reminders.create_smart_list")
    ])

    private static func proof(_ operation: String) -> JSON {
        .object([
            "operation": .string(operation),
            "proven": .bool(false),
            "reason": .string("Native Reminders parity effect and capture are not verified on this OS build"),
            "os_version": .string(ProcessInfo.processInfo.operatingSystemVersionString)
        ])
    }
}
