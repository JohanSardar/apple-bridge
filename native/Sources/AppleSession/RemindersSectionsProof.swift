#if LAB
import Foundation
import AppKit
import ApplicationServices
import EventKit
import RemindersStore
import MessagesSource
import SourceSupport

@MainActor
enum RemindersSectionsProof {
    static func run() async throws -> JSON {
        guard AXIsProcessTrusted(), EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { throw SourceFailure.permissionMissing }
        let store = EKEventStore()
        let run = UUID().uuidString
        let title = "[ab-test " + run + "]"
        let list = EKCalendar(for: .reminder, eventStore: store)
        guard let source = store.defaultCalendarForNewReminders()?.source ?? store.sources.first(where: { $0.sourceType == .calDAV }) else { throw SourceFailure.unavailable }
        list.source = source
        list.title = title
        try store.saveCalendar(list, commit: true)
        let identifier = list.calendarIdentifier
        let ledger = PrivateReport.directory.appendingPathComponent("reminders-sections-cleanup/" + run + ".json")
        do {
            try FileManager.default.createDirectory(at: ledger.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try PrivateReport.write(["list_id": identifier, "title": title, "cleanup_state": "pending"], to: ledger)
        } catch {
            try store.removeCalendar(list, commit: true)
            throw error
        }
        var checks: [String: JSON] = [:]
        var failure = ""
        do {
            let item = EKReminder(eventStore: store)
            item.calendar = list
            item.title = title + " item"
            try store.save(item, commit: true)
            let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.reminders") else { throw SourceFailure.unavailable }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            let app = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            try await Task.sleep(for: .seconds(1))
            let root = AXUIElementCreateApplication(app.processIdentifier)
            var ownedSelection: AXUIElement?
            @MainActor func fence() throws {
                guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
                if let ownedSelection {
                    var selected: CFTypeRef?
                    guard AXUIElementCopyAttributeValue(ownedSelection, kAXSelectedAttribute as CFString, &selected) == .success, (selected as? Bool) == true else { throw SourceFailure.conflict }
                }
            }
            @MainActor func value(_ element: AXUIElement, _ key: String) -> String {
                var result: CFTypeRef?
                guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return "" }
                return (result as? NSAttributedString)?.string ?? result as? String ?? ""
            }
            @MainActor func find(_ name: String, role: String? = nil) -> [AXUIElement] {
                var budget = 6000
                @MainActor func walk(_ element: AXUIElement) -> [AXUIElement] {
                    guard budget > 0 else { return [] }; budget -= 1
                    let actualRole = value(element, kAXRoleAttribute)
                    let labels = [value(element, kAXTitleAttribute), value(element, kAXValueAttribute), value(element, kAXDescriptionAttribute)]
                    if role == nil || actualRole == role {
                        if labels.contains(name), ![kAXApplicationRole, kAXWindowRole, kAXScrollAreaRole, kAXSplitGroupRole].contains(actualRole) { return [element] }
                    }
                    var result: [AXUIElement] = []
                    var children: CFTypeRef?
                    AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
                    for child in children as? [AXUIElement] ?? [] { result += walk(child) }
                    return result
                }
                return walk(root)
            }
            @MainActor func unique(_ name: String, role: String? = nil) throws -> AXUIElement {
                let matches = find(name, role: role)
                guard matches.count == 1 else { throw SourceFailure.conflict }
                return matches[0]
            }
            @MainActor func press(_ element: AXUIElement) throws {
                try fence()
                guard AXUIElementPerformAction(element, kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
            }
            @MainActor func key(_ code: CGKeyCode) throws {
                try fence()
                guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
                down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
            }
            @MainActor func point(_ element: AXUIElement) throws -> CGPoint {
                var position: CFTypeRef?, size: CFTypeRef?
                guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &position) == .success,
                      AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &size) == .success,
                      let position, let size, CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw SourceFailure.conflict }
                var origin = CGPoint.zero, dimensions = CGSize.zero
                guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin), AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions), dimensions.width > 0, dimensions.height > 0 else { throw SourceFailure.conflict }
                return CGPoint(x: origin.x + dimensions.width / 2, y: origin.y + dimensions.height / 2)
            }
            @MainActor func drag(_ from: AXUIElement, to: AXUIElement) async throws {
                let start = try point(from), end = try point(to)
                try fence()
                defer { CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: .left)?.post(tap: .cghidEventTap) }
                CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: start, mouseButton: .left)?.post(tap: .cghidEventTap)
                for index in 1...20 {
                    try fence()
                    let fraction = Double(index) / 20
                    let location = CGPoint(x: start.x + (end.x - start.x) * fraction, y: start.y + (end.y - start.y) * fraction)
                    CGEvent(mouseEventSource: nil, mouseType: .leftMouseDragged, mouseCursorPosition: location, mouseButton: .left)?.post(tap: .cghidEventTap)
                    try await Task.sleep(for: .milliseconds(30))
                }
                CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: .left)?.post(tap: .cghidEventTap)
                try await Task.sleep(for: .milliseconds(500))
            }
            var listElement = try unique(title)
            for _ in 0..<12 {
                if value(listElement, kAXRoleAttribute) == kAXRowRole { break }
                var parent: CFTypeRef?
                guard AXUIElementCopyAttributeValue(listElement, kAXParentAttribute as CFString, &parent) == .success, let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
                listElement = unsafeDowncast(parent, to: AXUIElement.self)
            }
            guard value(listElement, kAXRoleAttribute) == kAXRowRole else { throw SourceFailure.conflict }
            if AXUIElementPerformAction(listElement, kAXPressAction as CFString) != .success {
                try fence()
                guard AXUIElementSetAttributeValue(listElement, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else { throw SourceFailure.unsupported }
            }
            try await Task.sleep(for: .milliseconds(500))
            ownedSelection = listElement
            try fence()
            let names = [title + " first", title + " second"]
            for name in names {
                try press(try unique("File", role: kAXMenuBarItemRole))
                try press(try unique("New Section", role: kAXMenuItemRole))
                try await Task.sleep(for: .milliseconds(300))
                var focused: CFTypeRef?
                guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
                let field = unsafeDowncast(focused, to: AXUIElement.self)
                try fence()
                guard AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, name as CFString) == .success else { throw SourceFailure.unsupported }
                try key(36)
                try await Task.sleep(for: .milliseconds(400))
            }
            @MainActor func observed(_ reversed: Bool, member: Bool) async throws -> Bool {
                let deadline = ContinuousClock.now.advanced(by: .seconds(10))
                repeat {
                    for session in sessions {
                        let snapshot = try await session.read().snapshot
                        guard let row = snapshot.lists.values.first(where: { $0.identity.uuid?.uuidString == identifier || $0.identity.textIdentifier == identifier }),
                              let decoded = StoreSections.decode(order: row.sectionOrder, membership: row.sectionMembership) else { continue }
                        let sections = snapshot.sections.values.filter { $0.list == row.identity.key }
                        let ordered = decoded.order.compactMap { id in sections.first(where: { $0.identity.uuid == id || $0.identity.textIdentifier.flatMap(UUID.init(uuidString:)) == id })?.name }
                        let expected = reversed ? Array(names.reversed()) : names
                        guard ordered == expected else { continue }
                        if !member { return true }
                        if let itemID = UUID(uuidString: item.calendarItemIdentifier), let sectionID = decoded.membership[itemID], sections.first(where: { $0.identity.uuid == sectionID || $0.identity.textIdentifier.flatMap(UUID.init(uuidString:)) == sectionID })?.name == names[1] { return true }
                    }
                    try await Task.sleep(for: .milliseconds(100))
                } while ContinuousClock.now < deadline
                return false
            }
            checks["section_order_readback"] = .bool(try await observed(false, member: false))
            try await drag(try unique(item.title), to: unique(names[1]))
            checks["section_membership_readback"] = .bool(try await observed(false, member: true))
            try await drag(try unique(names[1]), to: unique(names[0]))
            checks["section_move_readback"] = .bool(try await observed(true, member: true))
        } catch { failure = String(describing: error) }
        store.reset()
        if let owned = store.calendar(withIdentifier: identifier), owned.title == title {
            try store.removeCalendar(owned, commit: true)
        }
        store.reset()
        checks["owned_list_cleanup"] = .bool(store.calendar(withIdentifier: identifier) == nil)
        try PrivateReport.write(["list_id": identifier, "title": title, "cleanup_state": checks["owned_list_cleanup"]?.bool == true ? "verified_clean" : "pending"], to: ledger)
        return .object(["passed": .bool(failure.isEmpty && checks.count == 4 && checks.values.allSatisfy(\.bool)), "checks": .object(checks), "reason": .string(failure)])
    }
}
#endif
