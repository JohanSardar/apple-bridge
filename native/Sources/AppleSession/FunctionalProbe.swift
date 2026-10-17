import Foundation
import EventKit
import Contacts
@preconcurrency import ApplicationServices

public enum FunctionalProbe {
    public static func run() -> [String: Any] {
        var report: [String: Any] = [:]
        let store = EKEventStore()
        report["calendar_status"] = statusName(EKEventStore.authorizationStatus(for: .event))
        report["reminders_status"] = statusName(EKEventStore.authorizationStatus(for: .reminder))
        let calendars = store.calendars(for: .event)
        report["calendar_count"] = calendars.count
        let now = Date()
        let predicate = store.predicateForEvents(withStart: now.addingTimeInterval(-30 * 86400), end: now.addingTimeInterval(30 * 86400), calendars: nil)
        report["event_count_60d"] = store.events(matching: predicate).count
        report["reminder_list_count"] = store.calendars(for: .reminder).count
        let contacts = CNContactStore()
        report["contacts_status"] = contactsStatusName(CNContactStore.authorizationStatus(for: .contacts))
        var contactCount = 0
        do {
            let request = CNContactFetchRequest(keysToFetch: [CNContactIdentifierKey as CNKeyDescriptor])
            try contacts.enumerateContacts(with: request) { _, _ in contactCount += 1 }
            report["contact_count"] = contactCount
        } catch {
            report["contact_error"] = "\((error as NSError).domain) \((error as NSError).code)"
        }
        let chatDatabase = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        report["chat_db_readable"] = FileManager.default.isReadableFile(atPath: chatDatabase) && (try? FileHandle(forReadingFrom: URL(fileURLWithPath: chatDatabase)).close()) != nil
        report["accessibility_trusted"] = AXIsProcessTrusted()
        return report
    }

    private static func statusName(_ status: EKAuthorizationStatus) -> String {
        switch status {
        case .fullAccess: "full_access"
        case .writeOnly: "write_only"
        case .denied: "denied"
        case .restricted: "restricted"
        case .notDetermined: "not_determined"
        @unknown default: "unknown"
        }
    }

    private static func contactsStatusName(_ status: CNAuthorizationStatus) -> String {
        switch status {
        case .authorized: "authorized"
        case .limited: "limited"
        case .denied: "denied"
        case .restricted: "restricted"
        case .notDetermined: "not_determined"
        @unknown default: "unknown"
        }
    }
}
