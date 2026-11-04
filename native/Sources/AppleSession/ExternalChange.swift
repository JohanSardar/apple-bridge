#if LAB
import Foundation
import EventKit
import Contacts
import NotesSource

@MainActor
enum ExternalChange {
    private static let tag = "[ab-test]"
    private static let fixtureCalendar = "Bridge Test"

    struct Failure: Error {
        let reason: String
    }

    static func run(_ request: [String: Any]) -> [String: Any] {
        do {
            let source = try text(request, "source")
            let action = try text(request, "action")
            switch source {
            case "reminders": return try reminders(action, request)
            case "calendar": return try calendar(action, request)
            case "contacts": return try contacts(action, request)
            case "notes": return try notes(action, request)
            default: throw Failure(reason: "source")
            }
        } catch let failure as Failure {
            return ["state": "failed", "reason": failure.reason]
        } catch {
            return ["state": "failed", "reason": String(describing: type(of: error))]
        }
    }

    private static func text(_ request: [String: Any], _ key: String) throws -> String {
        guard let value = request[key] as? String, !value.isEmpty, value.count <= 200,
              value.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x22 && $0.value != 0x5C && $0.value < 0x7F }) else { throw Failure(reason: "argument_" + key) }
        return value
    }

    private static func owned(_ request: [String: Any], _ key: String) throws -> String {
        let value = try text(request, key)
        guard value.hasPrefix(tag) else { throw Failure(reason: "untagged_" + key) }
        return value
    }

    private static func done(_ key: String) -> [String: Any] {
        ["state": "done", "key": key]
    }

    private static func reminders(_ action: String, _ request: [String: Any]) throws -> [String: Any] {
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { throw Failure(reason: "reminders_access") }
        let store = EKEventStore()
        switch action {
        case "create":
            let container = try owned(request, "container_title")
            guard let list = store.calendars(for: .reminder).first(where: { $0.title == container && $0.allowsContentModifications }) else { throw Failure(reason: "container") }
            let reminder = EKReminder(eventStore: store)
            reminder.title = try owned(request, "title")
            reminder.calendar = list
            try store.save(reminder, commit: true)
            return done(reminder.calendarItemIdentifier)
        case "update", "delete":
            let key = try text(request, "key")
            guard let reminder = store.calendarItem(withIdentifier: key) as? EKReminder, reminder.title?.hasPrefix(tag) == true else { throw Failure(reason: "target") }
            if action == "update" {
                reminder.title = try owned(request, "title")
                try store.save(reminder, commit: true)
            } else {
                try store.remove(reminder, commit: true)
            }
            return done(key)
        default: throw Failure(reason: "action")
        }
    }

    private static func calendar(_ action: String, _ request: [String: Any]) throws -> [String: Any] {
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { throw Failure(reason: "calendar_access") }
        let store = EKEventStore()
        switch action {
        case "create":
            guard let target = store.calendars(for: .event).first(where: { $0.title == fixtureCalendar && $0.allowsContentModifications }) else { throw Failure(reason: "container") }
            let event = EKEvent(eventStore: store)
            event.title = try owned(request, "title")
            event.calendar = target
            let start = Date(timeIntervalSinceReferenceDate: (Date().timeIntervalSinceReferenceDate / 3600).rounded(.up) * 3600 + 86400)
            event.startDate = start
            event.endDate = start.addingTimeInterval(1800)
            try store.save(event, span: .thisEvent, commit: true)
            return done(event.eventIdentifier ?? "")
        case "update", "delete":
            let key = try text(request, "key")
            guard let event = store.event(withIdentifier: key), event.title?.hasPrefix(tag) == true, event.calendar.title == fixtureCalendar else { throw Failure(reason: "target") }
            if action == "update" {
                event.title = try owned(request, "title")
                try store.save(event, span: .thisEvent, commit: true)
            } else {
                try store.remove(event, span: .thisEvent, commit: true)
            }
            return done(key)
        default: throw Failure(reason: "action")
        }
    }

    private static func contacts(_ action: String, _ request: [String: Any]) throws -> [String: Any] {
        guard CNContactStore.authorizationStatus(for: .contacts) == .authorized else { throw Failure(reason: "contacts_access") }
        let store = CNContactStore()
        switch action {
        case "create":
            let contact = CNMutableContact()
            contact.givenName = try owned(request, "title")
            contact.organizationName = fixtureCalendar
            let save = CNSaveRequest()
            save.add(contact, toContainerWithIdentifier: store.defaultContainerIdentifier())
            try store.execute(save)
            return done(contact.identifier)
        case "update", "delete":
            let key = try text(request, "key")
            let keys: [CNKeyDescriptor] = [CNContactGivenNameKey as NSString, CNContactOrganizationNameKey as NSString, CNContactJobTitleKey as NSString]
            let current = try store.unifiedContact(withIdentifier: key, keysToFetch: keys)
            guard current.givenName.hasPrefix(tag), current.organizationName == fixtureCalendar, let mutable = current.mutableCopy() as? CNMutableContact else { throw Failure(reason: "target") }
            let save = CNSaveRequest()
            if action == "update" {
                mutable.givenName = try owned(request, "title")
                save.update(mutable)
            } else {
                save.delete(mutable)
            }
            try store.execute(save)
            return done(key)
        default: throw Failure(reason: "action")
        }
    }

    private static func notes(_ action: String, _ request: [String: Any]) throws -> [String: Any] {
        switch action {
        case "create":
            let folder = try text(request, "container_title")
            let title = try owned(request, "title")
            let body = NotesScript.html(title + "\nexternal")
            let id = try NotesScript.run("set n to make new note at (first folder whose name is \(NotesScript.quote(folder))) with properties {body:\(NotesScript.quote(body))}\nreturn id of n")
            guard !id.isEmpty else { throw Failure(reason: "no_id") }
            return done(id)
        case "update", "delete":
            let key = try text(request, "key")
            let guardLine = "set n to note id \(NotesScript.quote(key))\nif (name of n) does not start with \(NotesScript.quote(tag)) then error \"untagged\" number 9001\n"
            if action == "update" {
                let body = NotesScript.html(try owned(request, "title") + "\nexternal revised")
                _ = try NotesScript.run(guardLine + "set body of n to \(NotesScript.quote(body))\nreturn \"ok\"")
            } else {
                _ = try NotesScript.run(guardLine + "delete n\nreturn \"ok\"")
            }
            return done(key)
        default: throw Failure(reason: "action")
        }
    }
}
#endif
