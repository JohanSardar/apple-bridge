#if LAB
import Foundation
import AppKit
import ApplicationServices
import Carbon
import CryptoKit
import EventKit
import CalendarSource
import SourceSupport

@MainActor
enum CalendarParityProof {
    static func run() async throws -> JSON {
        var checks: [String: JSON] = [:], reasons: [String: JSON] = [:], controls: [String: JSON] = [:]
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.iCal")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        checks["automation_granted"] = .bool(permission == noErr)
        checks["accessibility_granted"] = .bool(AXIsProcessTrusted())
        checks["calendar_granted"] = .bool(EKEventStore.authorizationStatus(for: .event) == .fullAccess)
        guard checks.values.allSatisfy(\.bool) else {
            reasons["permissions"] = .string("The signed Helper needs Calendar, Calendar Automation and Accessibility access in System Settings before this test can run.")
            return report(checks, reasons, controls)
        }
        let store = EKEventStore()
        let title = "[ab-test " + UUID().uuidString + "]"
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
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-calendar-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        var stage = "fixture"
        do {
            try store.save(event, span: .thisEvent, commit: true)
            let identifier = event.calendarItemIdentifier, external = event.calendarItemExternalIdentifier ?? ""
            let eventTitle = event.title ?? ""
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
            var error: NSDictionary?
            let script = "with timeout of 5 seconds\ntell application \"Calendar\"\nset matches to every calendar whose name is \"" + title + "\"\nif (count matches) is not 1 then error number -2700\nset eventsFound to every event of item 1 of matches whose summary is \"" + eventTitle + "\"\nif (count eventsFound) is not 1 then error number -2700\nshow item 1 of eventsFound\nactivate\nend tell\nend timeout"
            _ = NSAppleScript(source: script)?.executeAndReturnError(&error)
            guard error == nil else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(500))
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iCal").first else { throw SourceFailure.unavailable }
            let ui = CalendarFixtureUI(app: app, title: eventTitle)
            if !ui.hasOwnedInspector() {
                try ui.navigationInfo()
                try await Task.sleep(for: .milliseconds(400))
            }
            checks["owned_event_inspector"] = .bool(ui.hasOwnedInspector())
            try ui.fence()
            for name in ["travel time", "travel time:", "Travel Time", "Add Attachment…", "Add notes, URL or attachments", "Remove Attachment", "Accept", "Maybe", "Decline"] {
                controls[name] = .bool(!ui.find(name).isEmpty)
            }
            checks["invitation_response_readback"] = .bool(false)
            reasons["invitation"] = .string("This run has no owned incoming invitation. EventKit creates ordinary events and exposes attendees and response status as read-only; existing invitations were left untouched.")
            stage = "travel_time"
            try ui.travelPopup()
            try ui.pressAny(["15 minutes", "15 Minutes"], role: kAXMenuItemRole)
            try ui.key(48)
            checks["travel_time_readback"] = .bool(try await observed { current in
                let value = current["travel_time"] ?? .null
                return value == .number(900) || value == .number(15)
            })
            stage = "attachment_add"
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
            try ui.press("Open", role: kAXButtonRole)
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
            try ui.contextMenu(filename)
            try ui.pressAny(["Remove Attachment", "Delete Attachment"], role: kAXMenuItemRole)
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
            reasons[stage] = .string("Calendar could not verify " + stage + " on the test event. The action remains unavailable until this native check passes.")
        }
        store.reset()
        if let owned = store.calendar(withIdentifier: calendarID), owned.title == title { try store.removeCalendar(owned, commit: true) }
        store.reset()
        checks["owned_calendar_cleanup"] = .bool(store.calendar(withIdentifier: calendarID) == nil)
        return report(checks, reasons, controls)
    }

    private static func report(_ checks: [String: JSON], _ reasons: [String: JSON], _ controls: [String: JSON]) -> JSON {
        let required = ["invitation_response_readback", "travel_time_readback", "attachment_add_readback", "attachment_remove_readback", "owned_calendar_cleanup"]
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

    func hasOwnedInspector() -> Bool { find(title, role: kAXTextFieldRole).count + find(title, role: kAXTextAreaRole).count == 1 }

    func fence() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, hasOwnedInspector() else { throw SourceFailure.conflict }
    }

    func navigationInfo() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        try post(34, flags: .maskCommand)
    }

    private func post(_ code: CGKeyCode, flags: CGEventFlags) throws {
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

    func setFocused(_ text: String) throws {
        try fence()
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
