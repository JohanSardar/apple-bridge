#if LAB
import Foundation
import AppKit
import ApplicationServices
import Carbon
import EventKit
import RemindersStore
import SourceSupport

@MainActor
enum RemindersParityProof {
    static func run() async throws -> JSON {
        var checks: [String: JSON] = [:]
        var reasons: [String: JSON] = [:]
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.reminders")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        checks["automation_granted"] = .bool(permission == noErr)
        checks["accessibility_granted"] = .bool(AXIsProcessTrusted())
        checks["reminders_granted"] = .bool(EKEventStore.authorizationStatus(for: .reminder) == .fullAccess)
        guard checks.values.allSatisfy(\.bool) else {
            return .object(["passed": .bool(false), "checks": .object(checks), "reason": .string(permission != noErr ? "Reminders Automation is unavailable to the signed Helper; grant it in System Settings before retrying." : "The signed Helper needs Reminders and Accessibility access in System Settings.")])
        }
        let store = EKEventStore()
        let title = "[ab-test " + UUID().uuidString + "]"
        let list = EKCalendar(for: .reminder, eventStore: store)
        guard let source = store.defaultCalendarForNewReminders()?.source else { throw SourceFailure.unavailable }
        list.source = source
        list.title = title
        try store.saveCalendar(list, commit: true)
        let listID = list.calendarIdentifier
        let first = EKReminder(eventStore: store), second = EKReminder(eventStore: store)
        first.calendar = list; first.title = title + " parent"
        second.calendar = list; second.title = title + " child"
        var stage = "fixture"
        do {
            try store.save(first, commit: true)
            try store.save(second, commit: true)
            let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
            func observe(_ predicate: (TypedStoreSnapshot, StoreListRow, StoreReminderRow, StoreReminderRow) -> Bool) async throws -> Bool {
                for _ in 0..<60 {
                    for session in sessions {
                        let snapshot = try await session.read().snapshot
                        if let row = snapshot.lists.values.first(where: { $0.identity.uuid?.uuidString == listID || $0.identity.textIdentifier == listID }),
                           let parent = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == first.calendarItemIdentifier }),
                           let child = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == second.calendarItemIdentifier }), predicate(snapshot, row, parent, child) { return true }
                    }
                    try await Task.sleep(for: .milliseconds(100))
                }
                return false
            }
            func script(_ body: String) throws {
                var error: NSDictionary?
                let source = "with timeout of 5 seconds\ntell application \"Reminders\"\nset matches to every list whose name is \"" + title + "\"\nif (count matches) is not 1 then error number -2700\nset fixtureList to item 1 of matches\n" + body + "\nend tell\nend timeout"
                _ = NSAppleScript(source: source)?.executeAndReturnError(&error)
                guard error == nil else { throw SourceFailure.unavailable }
            }
            stage = "flag"
            try script("set flagged of every reminder of fixtureList whose name is \"" + first.title + "\" to true")
            checks["flag_readback"] = .bool(try await observe { _, _, parent, _ in parent.flagged == true })
            try script("set flagged of every reminder of fixtureList whose name is \"" + first.title + "\" to false")
            checks["flag_clear_readback"] = .bool(try await observe { _, _, parent, _ in parent.flagged == false })
            stage = "list_icon"
            try script("set emblem of fixtureList to \"star\"")
            checks["list_icon_readback"] = .bool(try await observe { _, row, _, _ in row.emblem == "star" })
            stage = "navigation"
            try script("show fixtureList\nactivate")
            try await Task.sleep(for: .seconds(1))
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.reminders").first else { throw SourceFailure.unavailable }
            let ui = RemindersFixtureUI(app: app, title: title)
            let expected = ["File", "Edit", "View", "New Section", "Indent Reminders", "Outdent Reminders", "Add Tags…", "Add Image", "New List…", "Show List Info", "Sort By", "Manual", "Add Image…", "Choose File…"]
            checks["owned_list_visible"] = .bool(!ui.find(title).isEmpty)
            for name in expected { checks["control_" + name] = .bool(!ui.find(name).isEmpty) }
            stage = "tags"
            let tag = "abtest" + UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
            try script("show (first reminder of fixtureList whose name is \"" + first.title + "\")\nactivate")
            try await Task.sleep(for: .milliseconds(300))
            let fields = ui.find(first.title, role: kAXTextFieldRole) + ui.find(first.title, role: kAXTextAreaRole)
            if fields.count == 1 {
                try ui.set(fields[0], text: first.title + " #" + tag)
                try ui.key(36)
                checks["tags_readback"] = .bool(try await observe { snapshot, _, parent, _ in StoreTags.names(for: parent.identity.key, snapshot: snapshot)?.contains(tag) == true })
            }
            stage = "subtasks"
            try script("show (first reminder of fixtureList whose name is \"" + second.title + "\")\nactivate")
            try await Task.sleep(for: .milliseconds(400))
            try await ui.menu("Edit", ["Indent Reminder", "Indent Reminders"])
            checks["indent_readback"] = .bool(try await observe { _, _, parent, child in child.parent == parent.identity.key })
            try await ui.menu("Edit", ["Outdent Reminder", "Outdent Reminders"])
            checks["outdent_readback"] = .bool(try await observe { _, _, _, child in child.parent == nil || child.parent == 0 })
        } catch {
            reasons[stage] = .string("The " + stage + " action could not be verified on the test list: " + String(describing: error))
        }
        store.reset()
        if let owned = store.calendar(withIdentifier: listID), owned.title == title { try store.removeCalendar(owned, commit: true) }
        store.reset()
        checks["owned_list_cleanup"] = .bool(store.calendar(withIdentifier: listID) == nil)
        let required = ["flag_readback", "flag_clear_readback", "list_icon_readback", "indent_readback", "outdent_readback", "tags_readback", "reorder_readback", "image_readback", "smart_list_readback", "owned_list_cleanup"]
        return .object(["passed": .bool(required.allSatisfy { checks[$0]?.bool == true }), "checks": .object(checks.filter { !$0.key.hasPrefix("control_") }), "controls": .object(checks.filter { $0.key.hasPrefix("control_") }), "reasons": .object(reasons)])
    }
}

@MainActor
final class RemindersFixtureUI {
    let app: NSRunningApplication
    let root: AXUIElement
    let title: String

    init(app: NSRunningApplication, title: String) {
        self.app = app; self.title = title
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.3)
    }

    func value(_ element: AXUIElement, _ key: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? ""
    }

    func find(_ name: String, role: String? = nil) -> [AXUIElement] {
        var budget = 6000
        func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget > 0 else { return [] }; budget -= 1
            let actualRole = value(element, kAXRoleAttribute)
            if (role == nil || role == actualRole), [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains(where: { value(element, $0) == name }) { return [element] }
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
            return (children as? [AXUIElement] ?? []).flatMap(walk)
        }
        return walk(root)
    }

    func press(_ name: String, role: String) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        let matches = find(name, role: role)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func set(_ element: AXUIElement, text: String) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        guard AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success,
              AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, text as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func key(_ code: CGKeyCode, flags: CGEventFlags = []) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
    }

    func menu(_ menu: String, _ items: [String]) async throws {
        try press(menu, role: kAXMenuBarItemRole)
        try await Task.sleep(for: .milliseconds(100))
        let available = items.filter { find($0, role: kAXMenuItemRole).count == 1 }
        guard available.count == 1 else { throw SourceFailure.unsupported }
        try press(available[0], role: kAXMenuItemRole)
        try await Task.sleep(for: .milliseconds(300))
    }
}
#endif
