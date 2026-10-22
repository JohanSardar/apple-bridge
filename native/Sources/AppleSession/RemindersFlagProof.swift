#if LAB
import Foundation
import AppKit
import EventKit
import Carbon
import RemindersStore
import SourceSupport

@MainActor
enum RemindersFlagProof {
    static func run(tag: String, itemIdentifier: String, listIdentifier: String) async throws -> JSON {
        guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\""), !tag.contains("\\"), !tag.contains("\n") else { throw SourceFailure.invalid }
        let eventStore = EKEventStore()
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess,
              let reminder = eventStore.calendarItem(withIdentifier: itemIdentifier) as? EKReminder,
              reminder.calendar.calendarIdentifier == listIdentifier,
              reminder.calendar.title == "Bridge Test " + tag,
              reminder.title == tag + " identity" else { throw SourceFailure.conflict }
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.reminders")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        guard permission == noErr else {
            return .object(["status": .string("limited"), "reason": .string("Reminders Apple Events authorization unavailable without prompting"), "permission_code": .number(Double(permission))])
        }
        let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
        for session in sessions { _ = try await session.read() }
        func setFlag(_ value: Bool) throws {
            let script = "with timeout of 3 seconds\ntell application \"Reminders\"\nset fixtureLists to every list whose name is \"Bridge Test " + tag + "\"\nif (count fixtureLists) is not 1 then error number -2700\nset fixtureItems to every reminder of item 1 of fixtureLists whose name is \"" + tag + " identity\"\nif (count fixtureItems) is not 1 then error number -2700\nset flagged of item 1 of fixtureItems to " + (value ? "true" : "false") + "\nend tell\nend timeout"
            var error: NSDictionary?
            _ = NSAppleScript(source: script)?.executeAndReturnError(&error)
            guard error == nil else { throw SourceFailure.unavailable }
        }
        func matches(_ expected: Bool) async throws -> Bool {
            let deadline = ProcessInfo.processInfo.systemUptime + 3
            repeat {
                var matches: [StoreReminderRow] = []
                for session in sessions {
                    matches += try await session.read().snapshot.reminders.values.filter { $0.calendarItemIdentifier == itemIdentifier && $0.identity.uuid?.uuidString == itemIdentifier }
                }
                if matches.count == 1, matches[0].flagged == expected { return true }
                try await Task.sleep(for: .milliseconds(20))
            } while ProcessInfo.processInfo.systemUptime < deadline
            return false
        }
        defer { try? setFlag(false) }
        try setFlag(true)
        let present = try await matches(true)
        try setFlag(false)
        let cleared = try await matches(false)
        return .object(["status": .string("ready"), "store_flag_true": .bool(present), "store_flag_cleared": .bool(cleared)])
    }
}
#endif
