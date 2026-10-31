#if LAB
import Foundation
import AppKit
import ApplicationServices
import CoreLocation
import CryptoKit
import EventKit
import CalendarSource
import SourceSupport

@MainActor
enum CalendarParityProof {
    static func run() async throws -> JSON {
        var checks: [String: JSON] = [:], reasons: [String: JSON] = [:], controls: [String: JSON] = [:]
        checks["accessibility_granted"] = .bool(AXIsProcessTrusted())
        checks["calendar_granted"] = .bool(EKEventStore.authorizationStatus(for: .event) == .fullAccess)
        guard checks.values.allSatisfy(\.bool) else {
            reasons["permissions"] = .string("Calendar full access and Accessibility must already be enabled for the signed Helper. This check never requests access.")
            return report(checks, reasons, controls)
        }
        let store = EKEventStore()
        let title = "[ab-test] " + UUID().uuidString
        let calendar = EKCalendar(for: .event, eventStore: store)
        guard let source = store.defaultCalendarForNewEvents?.source else { throw SourceFailure.unavailable }
        calendar.title = title; calendar.source = source
        try store.saveCalendar(calendar, commit: true)
        let calendarID = calendar.calendarIdentifier
        defer {
            store.reset()
            if let owned = store.calendar(withIdentifier: calendarID), owned.title == title { try? store.removeCalendar(owned, commit: true) }
        }
        let event = EKEvent(eventStore: store)
        event.calendar = calendar; event.title = title + " event"
        event.startDate = Date().addingTimeInterval(86400); event.endDate = event.startDate.addingTimeInterval(3600)
        let location = EKStructuredLocation(title: "[ab-test] location")
        location.geoLocation = CLLocation(latitude: 41.88, longitude: -87.63)
        location.radius = 100
        event.structuredLocation = location
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-calendar-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        var stage = "fixture"
        do {
            try store.save(event, span: .thisEvent, commit: true)
            let identifier = event.calendarItemIdentifier, external = event.calendarItemExternalIdentifier ?? ""
            let eventTitle = event.title ?? ""
            let eventStart = event.startDate!, eventEnd = event.endDate!
            let database = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb")
            func row() throws -> [String: JSON]? {
                try CalendarStoreSnapshot.read(path: database.path).event(identifier: identifier, external: external, calendar: calendarID, detached: false)
            }
            @MainActor func observed(_ predicate: ([String: JSON]) -> Bool) async throws -> Bool {
                for _ in 0..<60 {
                    if let current = try row(), predicate(current) { return true }
                    try await Task.sleep(for: .milliseconds(100))
                }
                return false
            }
            stage = "show_owned_event"
            store.reset()
            let saved = store.calendarItem(withIdentifier: identifier) as? EKEvent
            checks["structured_location_readback"] = .bool(saved?.structuredLocation?.title == location.title && saved?.structuredLocation?.geoLocation?.coordinate.latitude == 41.88 && saved?.structuredLocation?.geoLocation?.coordinate.longitude == -87.63 && saved?.structuredLocation?.radius == 100)
            let identities = ownIdentities(store: store)
            controls["own_account_identity_count"] = .number(Double(identities.count))
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.iCal") else { throw SourceFailure.unavailable }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            let app = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            try await Task.sleep(for: .seconds(1))
            let ui = CalendarFixtureUI(app: app, title: eventTitle)
            try await ui.openOwnedEvent()
            checks["owned_event_inspector"] = .bool(ui.hasOwnedInspector())
            try ui.fence()
            for name in ["travel time", "travel time:", "Travel Time", "Add Attachment…", "Add notes, URL or attachments", "Remove Attachment", "Accept", "Maybe", "Decline", "Add Invitees", "Add Invitees…", "Add Alert, Repeat, or Travel Time", "None", "Done", "Send"] {
                controls[name] = .bool(!ui.find(name).isEmpty)
            }
            checks["invitation_response_readback"] = .bool(false)
            reasons["invitation"] = .string("This run has no owned incoming invitation. EventKit creates ordinary events and exposes attendees and response status as read-only; existing invitations were left untouched.")
            do {
                stage = "travel_time"
                try await ui.revealTravel()
                try ui.travelPopup()
                try ui.pressAny(["15 minutes", "15 Minutes"], role: kAXMenuItemRole)
                try ui.key(48)
                try await ui.commit()
                checks["travel_time_readback"] = .bool(try await observed { current in
                    let value = current["travel_time"] ?? .null
                    return value == .number(900) || value == .number(15)
                })
            } catch {
                reasons["travel_time"] = .string("Calendar could not change travel time on the test event. Keep Calendar open and retry this check.")
            }
            do {
                stage = "attachment_add"
                try await ui.openOwnedEvent()
                let filename = "ab-test-" + UUID().uuidString + ".txt"
                let file = root.appendingPathComponent(filename)
                let bytes = Data("[ab-test] calendar attachment".utf8)
                try bytes.write(to: file)
                let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
                if ui.find("Add Attachment…", role: kAXButtonRole).isEmpty {
                    try await ui.menu("Edit", ["Attach File…", "Add Attachment…", "Attach File..."])
                } else { try ui.press("Add Attachment…", role: kAXButtonRole) }
                try await Task.sleep(for: .milliseconds(200))
                try ui.key(5, flags: [.maskCommand, .maskShift])
                try await Task.sleep(for: .milliseconds(200))
                try ui.setFocused(file.path)
                try ui.key(36)
                try await Task.sleep(for: .milliseconds(200))
                try ui.dialogPress("Open")
                try await ui.commit()
                let reader = CalendarStoreEnrichment()
                let observation: [String: JSON] = ["event": .object(["collection": .string("calendar.event"), "native_key": .string(identifier), "data": .object(["calendar_id": .object(["$ref": .object(["native_key": .string(calendarID)])])])])]
                let lookups: [String: JSON] = [identifier: .object(["external": .string(external)])]
                var attached = false
                for _ in 0..<60 {
                    let captured = await reader.enrich(observation, lookups: lookups, force: true)
                    attached = captured["event"]?["data"]["attachments"].array.contains { $0["filename"].string == filename && $0["blob"]["digest"].string == digest } == true
                    if attached { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
                checks["attachment_add_readback"] = .bool(attached)
                stage = "attachment_remove"
                try await ui.openOwnedEvent()
                try ui.contextMenu(filename)
                try ui.pressAny(["Remove Attachment", "Delete Attachment"], role: kAXMenuItemRole)
                try await ui.commit()
                var removed = false
                for _ in 0..<60 {
                    let captured = await reader.enrich(observation, lookups: lookups, force: true)
                    let current = try row()
                    removed = captured["event"]?["data"]["attachments"] == .array([]) && current != nil
                    if removed { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
                checks["attachment_remove_readback"] = .bool(removed)
            } catch {
                reasons["attachments"] = .string("Calendar could not change attachments on the test event. Keep Calendar open and retry this check.")
            }
            stage = "invitation"
            if let attendee = identities.sorted().last, let organizer = identities.sorted().first {
                try await ui.openOwnedEvent()
                let invitationTitle = title + " invitation"
                let invitationUI = CalendarFixtureUI(app: app, title: invitationTitle)
                let date = DateFormatter()
                date.locale = Locale(identifier: "en_US_POSIX"); date.timeZone = TimeZone(secondsFromGMT: 0); date.dateFormat = "yyyyMMdd'T'HHmmss'Z'"
                let lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//AppleBridge//Calendar parity//EN", "METHOD:REQUEST", "BEGIN:VEVENT", "UID:" + UUID().uuidString, "DTSTAMP:" + date.string(from: Date()), "DTSTART:" + date.string(from: eventStart), "DTEND:" + date.string(from: eventEnd), "SUMMARY:" + invitationTitle, "ORGANIZER:mailto:" + organizer, "ATTENDEE;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:" + attendee, "END:VEVENT", "END:VCALENDAR", ""]
                let invitationFile = root.appendingPathComponent("[ab-test]-invitation.ics")
                try Data(lines.joined(separator: "\r\n").utf8).write(to: invitationFile)
                try await ui.importInvitation(invitationFile, calendarTitle: title)
                store.reset()
                guard let importedCalendar = store.calendar(withIdentifier: calendarID) else { throw SourceFailure.unavailable }
                let query = store.predicateForEvents(withStart: eventStart.addingTimeInterval(-1), end: eventEnd.addingTimeInterval(1), calendars: [importedCalendar])
                let invitations = store.events(matching: query).filter { $0.title == invitationTitle }
                guard invitations.count == 1, let invitation = invitations.first,
                      let participants = invitation.attendees, participants.count == 1,
                      participants[0].isCurrentUser,
                      participants[0].url.absoluteString.lowercased() == "mailto:" + attendee else { throw SourceFailure.conflict }
                checks["invitation_self_attendee_only"] = .bool(true)
                let invitationID = invitation.calendarItemIdentifier
                let before = participants[0].participantStatus
                try await invitationUI.openOwnedEvent()
                for name in ["Accept", "Maybe", "Decline", "Reply", "OK"] { controls[name] = .bool(!invitationUI.find(name).isEmpty) }
                try invitationUI.press("Accept", role: kAXButtonRole)
                for _ in 0..<60 {
                    store.reset()
                    if let current = store.calendarItem(withIdentifier: invitationID) as? EKEvent,
                       current.title == invitationTitle, current.calendar.calendarIdentifier == calendarID,
                       let attendees = current.attendees, attendees.count == 1,
                       attendees[0].isCurrentUser, attendees[0].participantStatus == .accepted, before != .accepted {
                        checks["invitation_response_readback"] = .bool(true)
                        reasons.removeValue(forKey: "invitation")
                        break
                    }
                    try await Task.sleep(for: .milliseconds(200))
                }
            } else {
                reasons["invitation"] = .string("EventKit could not identify Johan's own account attendee address. No invitation was created or sent.")
            }
        } catch {
            reasons[stage] = .string("Calendar could not verify " + stage + " on the test event. Keep Calendar open and retry this check.")
            if let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iCal").first {
                let ui = CalendarFixtureUI(app: app, title: title + " event")
                for name in [title + " event", "travel time", "travel time:", "Travel Time", "Add Alert, Repeat, or Travel Time", "Add Attachment…", "Add notes, URL or attachments", "Accept", "Maybe", "Decline", "None", "Done", "Send", "Open", "Cancel"] {
                    controls[name == title + " event" ? "owned_title_visible" : name] = .number(Double(ui.find(name).count))
                }
            }
        }
        if let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iCal").first {
            let ui = CalendarFixtureUI(app: app, title: title + " event")
            if ui.find("Cancel", role: kAXButtonRole).count == 1 { try? ui.press("Cancel", role: kAXButtonRole) }
        }
        store.reset()
        if let owned = store.calendar(withIdentifier: calendarID), owned.title == title { try store.removeCalendar(owned, commit: true) }
        store.reset()
        checks["owned_calendar_cleanup"] = .bool(store.calendar(withIdentifier: calendarID) == nil)
        return report(checks, reasons, controls)
    }

    private static func ownIdentities(store: EKEventStore) -> Set<String> {
        let now = Date()
        let predicate = store.predicateForEvents(withStart: now.addingTimeInterval(-180 * 86400), end: now.addingTimeInterval(180 * 86400), calendars: nil)
        var identities: Set<String> = []
        for event in store.events(matching: predicate) {
            for participant in (event.attendees ?? []) + [event.organizer].compactMap({ $0 }) where participant.isCurrentUser && participant.url.scheme?.lowercased() == "mailto" {
                identities.insert(String(participant.url.absoluteString.dropFirst(7)).removingPercentEncoding?.lowercased() ?? "")
            }
        }
        return identities.filter { !$0.isEmpty && !$0.contains("\n") && !$0.contains("\r") }
    }

    private static func report(_ checks: [String: JSON], _ reasons: [String: JSON], _ controls: [String: JSON]) -> JSON {
        let required = ["accessibility_granted", "calendar_granted", "structured_location_readback", "invitation_response_readback", "travel_time_readback", "attachment_add_readback", "attachment_remove_readback", "owned_calendar_cleanup"]
        return .object(["passed": .bool(required.allSatisfy { checks[$0]?.bool == true }), "checks": .object(checks), "reasons": .object(reasons), "controls": .object(controls)])
    }
}

@MainActor
final class CalendarFixtureUI {
    let app: NSRunningApplication
    let root: AXUIElement
    let title: String

    init(app: NSRunningApplication, title: String) {
        self.app = app; self.title = title
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.1)
    }

    func value(_ element: AXUIElement, _ key: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? ""
    }

    func find(_ name: String, role: String? = nil) -> [AXUIElement] {
        var budget = 3000
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget > 0, ContinuousClock.now < deadline else { return [] }; budget -= 1
            if (role == nil || value(element, kAXRoleAttribute) == role), [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains(where: { value(element, $0) == name }) { return [element] }
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
            return (children as? [AXUIElement] ?? []).flatMap(walk)
        }
        return walk(root)
    }

    func openOwnedEvent() async throws {
        app.activate()
        try await Task.sleep(for: .milliseconds(200))
        if hasOwnedInspector() { return }
        try post(3, flags: .maskCommand)
        try await Task.sleep(for: .milliseconds(150))
        try setFocused(title, requireInspector: false)
        try post(36, flags: [])
        try await Task.sleep(for: .seconds(1))
        let matches = find(title)
        guard !matches.isEmpty else { throw SourceFailure.unavailable }
        let result = matches.first { value($0, kAXRoleAttribute) != kAXTextFieldRole } ?? matches[0]
        var position: CFTypeRef?, size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(result, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(result, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size else { throw SourceFailure.unsupported }
        var point = CGPoint.zero, dimensions = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &point),
              AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions) else { throw SourceFailure.unsupported }
        point.x += dimensions.width / 2; point.y += dimensions.height / 2
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else { throw SourceFailure.unavailable }
            event.setIntegerValueField(.mouseEventClickState, value: 2)
            event.post(tap: .cghidEventTap)
        }
        try await Task.sleep(for: .milliseconds(400))
        if !hasOwnedInspector() { try post(34, flags: .maskCommand); try await Task.sleep(for: .milliseconds(400)) }
        try fence()
    }

    func commit() async throws {
        try fence()
        if find("Done", role: kAXButtonRole).count == 1 { try press("Done", role: kAXButtonRole) }
        else { try post(53, flags: []) }
        try await Task.sleep(for: .milliseconds(500))
    }

    func revealTravel() async throws {
        try fence()
        if !find("travel time", role: kAXStaticTextRole).isEmpty || !find("travel time:", role: kAXStaticTextRole).isEmpty { return }
        for name in ["Add Alert, Repeat, or Travel Time", "Add Alert, Repeat or Travel Time"] {
            let matches = find(name)
            if matches.count == 1 {
                guard AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
                try await Task.sleep(for: .milliseconds(200))
                return
            }
        }
    }

    func importInvitation(_ file: URL, calendarTitle: String) async throws {
        try fence()
        try await menu("File", ["Import…", "Import..."])
        try await Task.sleep(for: .milliseconds(200))
        try post(5, flags: [.maskCommand, .maskShift])
        try await Task.sleep(for: .milliseconds(200))
        try setFocused(file.path, requireInspector: false)
        try post(36, flags: [])
        try await Task.sleep(for: .milliseconds(200))
        try dialogPress("Import")
        try await Task.sleep(for: .milliseconds(500))
        let popups = findRole(kAXPopUpButtonRole)
        guard popups.count == 1, AXUIElementPerformAction(popups[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
        try await Task.sleep(for: .milliseconds(100))
        let calendars = find(calendarTitle, role: kAXMenuItemRole)
        guard calendars.count == 1, AXUIElementPerformAction(calendars[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
        try dialogPress("OK")
        try await Task.sleep(for: .seconds(1))
    }

    func findRole(_ role: String) -> [AXUIElement] {
        var budget = 3000
        func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget > 0 else { return [] }; budget -= 1
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
            let descendants = (children as? [AXUIElement] ?? []).flatMap(walk)
            return (value(element, kAXRoleAttribute) == role ? [element] : []) + descendants
        }
        return walk(root)
    }

    func dialogPress(_ name: String) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        let matches = find(name, role: kAXButtonRole)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func hasOwnedInspector() -> Bool {
        let fields = find(title, role: kAXTextFieldRole).filter { value($0, kAXSubroleAttribute) != kAXSearchFieldSubrole }
        return fields.count + find(title, role: kAXTextAreaRole).count == 1
    }

    func fence() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, hasOwnedInspector() else { throw SourceFailure.conflict }
    }

    func navigationInfo() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        try post(34, flags: .maskCommand)
    }

    func post(_ code: CGKeyCode, flags: CGEventFlags) throws {
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
    }

    func key(_ code: CGKeyCode, flags: CGEventFlags = []) throws { try fence(); try post(code, flags: flags) }

    func press(_ name: String, role: String) throws {
        try fence()
        let matches = find(name, role: role)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func pressAny(_ names: [String], role: String) throws {
        let available = names.filter { find($0, role: role).count == 1 }
        guard available.count == 1 else { throw SourceFailure.unsupported }
        try press(available[0], role: role)
    }

    func menu(_ name: String, _ items: [String]) async throws {
        try press(name, role: kAXMenuBarItemRole)
        try await Task.sleep(for: .milliseconds(100))
        try pressAny(items, role: kAXMenuItemRole)
    }

    func setFocused(_ text: String, requireInspector: Bool = true) throws {
        if requireInspector { try fence() }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
        let field = unsafeDowncast(focused, to: AXUIElement.self)
        guard value(field, kAXRoleAttribute) == kAXTextFieldRole, AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, text as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func travelPopup() throws {
        try fence()
        let labels = ["travel time", "travel time:", "Travel Time", "Travel Time:"].flatMap { find($0, role: kAXStaticTextRole) }
        guard labels.count == 1 else { throw SourceFailure.unsupported }
        var current = labels[0]
        for _ in 0..<3 {
            var parent: CFTypeRef?
            guard AXUIElementCopyAttributeValue(current, kAXParentAttribute as CFString, &parent) == .success, let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
            current = unsafeDowncast(parent, to: AXUIElement.self)
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(current, kAXChildrenAttribute as CFString, &children)
            let popups = (children as? [AXUIElement] ?? []).filter { value($0, kAXRoleAttribute) == kAXPopUpButtonRole }
            if popups.count == 1 {
                guard AXUIElementPerformAction(popups[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
                return
            }
        }
        throw SourceFailure.unsupported
    }

    func contextMenu(_ name: String) throws {
        try fence()
        let matches = find(name)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXShowMenuAction as CFString) == .success else { throw SourceFailure.unsupported }
    }
}
#endif
