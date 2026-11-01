import Foundation
import AppKit
import ApplicationServices
import SQLite3
import SourceSupport

@MainActor
public enum ClockTimers {
    private static func failure() -> any Error { ClockTimerFailure() }

    public static func durationComponent(_ value: Any?) -> Int? {
        let number: Int?
        if let value = value as? NSNumber {
            number = value.doubleValue == Double(value.intValue) ? value.intValue : nil
        } else if let value = value as? String, let prefix = value.split(whereSeparator: \.isWhitespace).first {
            number = Int(prefix)
        } else { number = nil }
        guard let number, (0...59).contains(number) else { return nil }
        return number
    }

    public static func adjustDurationComponent(to target: Int, value: () -> Int?, adjust: (Bool) -> Bool) throws {
        guard (0...59).contains(target) else { throw failure() }
        for _ in 0...60 {
            guard let current = value(), (0...59).contains(current) else { throw failure() }
            if current == target { return }
            guard adjust(current < target) else { throw failure() }
        }
        throw failure()
    }

    public static func exchange(_ request: Data) throws -> Data {
        guard request.count <= 32768, let value = try JSONSerialization.jsonObject(with: request) as? [String: Any], let operation = value["operation"] as? String else { throw failure() }
        let before = try snapshot()
        guard AXIsProcessTrusted() else { throw failure() }
        var result: [String: Any] = ["version": 1, "operation": operation, "capabilities": ["timers", "start_timer", "delete_timer"]]
        switch operation {
        case "capabilities", "timers": break
        case "recent_timers": result["timers"] = try snapshot(recent: true)
        case "start_timer":
            guard let label = value["label"] as? String, label.hasPrefix("[ab-test]"), label.utf8.count <= 256, let duration = value["duration"] as? Int, (1...86399).contains(duration), !before.contains(where: { $0["label"] as? String == label }) else { throw failure() }
            let recent = try snapshot(recent: true)
            guard !recent.contains(where: { $0["label"] as? String == label }) else { throw failure() }
            let ui = try ClockTimerUI()
            try ui.create(label: label, duration: duration)
            let rows = try awaitSnapshot { rows in rows.filter { $0["label"] as? String == label && $0["duration"] as? Int == duration && $0["running"] as? Bool == true }.count == 1 }
            result["timer_running"] = true
            result["duration"] = duration
            result["label"] = label
            result["timers"] = rows
        case "delete_timer":
            guard let timer = value["timer"] as? [String: Any], let identifier = timer["id"] as? String, let label = timer["label"] as? String, label.hasPrefix("[ab-test]") else { throw failure() }
            if let existing = before.first(where: { $0["id"] as? String == identifier }) {
                guard existing["label"] as? String == label, before.filter({ $0["label"] as? String == label }).count == 1 else { throw failure() }
                try ClockTimerUI().remove(label: label)
            }
            result["timers"] = try awaitSnapshot { !$0.contains(where: { $0["id"] as? String == identifier }) }
            if try snapshot(recent: true).contains(where: { $0["label"] as? String == label }) {
                try ClockTimerUI().removeRecent(label: label)
                _ = try awaitSnapshot(recent: true) { !$0.contains(where: { $0["label"] as? String == label }) }
            }
        case "delete_recent_timer":
            guard let timer = value["timer"] as? [String: Any], let identifier = timer["id"] as? String, let label = timer["label"] as? String, label.hasPrefix("[ab-test]"), identifier.hasPrefix("recent:") else { throw failure() }
            let recent = try snapshot(recent: true)
            if let existing = recent.first(where: { $0["id"] as? String == identifier }) {
                guard existing["label"] as? String == label, recent.filter({ $0["label"] as? String == label }).count == 1 else { throw failure() }
                try ClockTimerUI().removeRecent(label: label)
            }
            result["timers"] = try awaitSnapshot(recent: true) { !$0.contains(where: { $0["id"] as? String == identifier }) }
        default: throw failure()
        }
        if result["timers"] == nil { result["timers"] = try snapshot() }
        let output = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        guard output.count <= 32768 else { throw failure() }
        return output
    }

    private static func awaitSnapshot(recent: Bool = false, _ accepts: ([[String: Any]]) -> Bool) throws -> [[String: Any]] {
        let deadline = Date().addingTimeInterval(10)
        repeat {
            let rows = try snapshot(recent: recent)
            if accepts(rows) { return rows }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw failure()
    }

    public static func snapshot(at location: URL? = nil, recent: Bool = false) throws -> [[String: Any]] {
        let path = (location ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.mobiletimerd/local.sqlite")).path
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if let database { sqlite3_close(database) }; throw failure() }
        defer { sqlite3_close(database) }
        sqlite3_busy_timeout(database, 1000)
        var statement: OpaquePointer?
        let query = recent ? "SELECT 'recent:' || Z_PK, COALESCE(ZTITLE, ''), ZDURATION, 1 FROM ZMTCDTIMERDURATION WHERE ZVALIDRECENT = 1 ORDER BY Z_PK" : "SELECT hex(ZMTID), COALESCE(ZTITLE, ''), ZDURATION, ZSTATE FROM ZMTCDTIMER WHERE ZSTATE IN (2, 3) ORDER BY Z_PK"
        guard sqlite3_prepare_v2(database, query, -1, &statement, nil) == SQLITE_OK else { throw failure() }
        defer { sqlite3_finalize(statement) }
        var rows: [[String: Any]] = []
        var step = sqlite3_step(statement)
        while step == SQLITE_ROW {
            guard let id = sqlite3_column_text(statement, 0), let label = sqlite3_column_text(statement, 1) else { throw failure() }
            rows.append(["id": String(cString: id), "label": String(cString: label), "duration": Int(sqlite3_column_double(statement, 2)), "running": sqlite3_column_int(statement, 3) == 3])
            step = sqlite3_step(statement)
        }
        guard step == SQLITE_DONE else { throw failure() }
        return rows
    }
}

private struct ClockTimerFailure: Error, CustomStringConvertible {
    var step: String = "read or update its timer"
    var description: String { "Clock could not \(step). Open Clock, allow Apple Bridge Accessibility access in System Settings, and retry. Only tagged test timers can be changed." }
}

@MainActor
private struct ClockTimerUI {
    let app: NSRunningApplication
    let root: AXUIElement

    init() throws {
        if NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.clock").isEmpty {
            NSWorkspace.shared.open(URL(fileURLWithPath: "/System/Applications/Clock.app"))
            RunLoop.current.run(until: Date().addingTimeInterval(1))
        }
        guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.clock").first else { throw ClockTimerFailure() }
        self.app = app
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.5)
        app.activate()
        for window in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] {
            _ = AXUIElementSetAttributeValue(window, kAXMinimizedAttribute as CFString, kCFBooleanFalse)
            _ = AXUIElementPerformAction(window, kAXRaiseAction as CFString)
            _ = AXUIElementSetAttributeValue(root, kAXFocusedWindowAttribute as CFString, window)
        }
        try press(find(root, role: kAXRadioButtonRole).filter { name($0) == "Timers" || name($0) == "Timer" }, step: "open the Timers tab")
    }

    private func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
        var result: CFTypeRef?
        AXUIElementCopyAttributeValue(element, key as CFString, &result)
        return result
    }

    private func name(_ element: AXUIElement) -> String {
        for key in [kAXTitleAttribute, kAXDescriptionAttribute] {
            if let value = attribute(element, key) as? String, !value.isEmpty { return value }
        }
        return ""
    }

    private func find(_ element: AXUIElement, role: String? = nil, identifier: String? = nil) -> [AXUIElement] {
        var budget = 2000
        var visited: [AXUIElement] = []
        @MainActor func visit(_ item: AXUIElement, depth: Int) -> [AXUIElement] {
            guard depth < 20, budget > 0 else { return [] }
            for previous in visited { if CFEqual(previous, item) { return [] } }
            budget -= 1
            visited.append(item)
            var result: [AXUIElement] = []
            let actualRole = attribute(item, kAXRoleAttribute) as? String
            let actualIdentifier = attribute(item, kAXIdentifierAttribute) as? String
            if (role == nil || actualRole == role) && (identifier == nil || actualIdentifier == identifier) { result.append(item) }
            for key in [kAXChildrenAttribute, kAXVisibleChildrenAttribute, kAXContentsAttribute, kAXRowsAttribute, kAXWindowsAttribute] {
                for child in attribute(item, key) as? [AXUIElement] ?? [] { result += visit(child, depth: depth + 1) }
            }
            for key in [kAXFocusedUIElementAttribute, kAXShownMenuUIElementAttribute] {
                if let related = attribute(item, key), CFGetTypeID(related) == AXUIElementGetTypeID() {
                    result += visit(unsafeDowncast(related, to: AXUIElement.self), depth: depth + 1)
                }
            }
            return result
        }
        return visit(element, depth: 0)
    }

    private func press(_ elements: [AXUIElement], step: String = "press the timer control") throws {
        guard elements.count == 1, AXUIElementPerformAction(elements[0], kAXPressAction as CFString) == .success else { throw ClockTimerFailure(step: step) }
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }

    private func focus(_ element: AXUIElement) throws {
        app.activate()
        _ = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        _ = AXUIElementSetAttributeValue(element, kAXSelectedAttribute as CFString, kCFBooleanTrue)
        if AXUIElementPerformAction(element, kAXPressAction as CFString) != .success { try click(element, button: .left) }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }

    private func click(_ element: AXUIElement, button: CGMouseButton) throws {
        app.activate()
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        guard let position = attribute(element, kAXPositionAttribute), let size = attribute(element, kAXSizeAttribute), CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw ClockTimerFailure(step: "locate the timer control") }
        var point = CGPoint.zero, extent = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &point), AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &extent), extent.width > 0, extent.height > 0 else { throw ClockTimerFailure(step: "locate the timer control") }
        point.x += extent.width / 2
        point.y += extent.height / 2
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, let source = CGEventSource(stateID: .privateState) else { throw ClockTimerFailure(step: "bring its timer controls to the foreground") }
        let types: [CGEventType] = button == .right ? [.rightMouseDown, .rightMouseUp] : [.leftMouseDown, .leftMouseUp]
        for type in types {
            guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: button) else { throw ClockTimerFailure(step: "select its timer control") }
            event.flags = []
            event.post(tap: .cghidEventTap)
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }

    func create(label: String, duration: Int) throws {
        var started = false
        defer {
            if !started { try? dismissTimerForm() }
        }
        if find(root, identifier: "TimePicker").isEmpty {
            let add = find(root, role: kAXMenuButtonRole).filter { name($0) == "Add" || name($0) == "Add a timer" }
            try press(add, step: "open the timer menu")
            try press(find(root, identifier: "addTimer:"), step: "choose a new timer")
        }
        func slider(_ index: Int) -> AXUIElement? {
            let picker = find(root, identifier: "TimePicker")
            guard picker.count == 1 else { return nil }
            let sliders = find(picker[0], role: kAXSliderRole)
            return sliders.count == 3 ? sliders[index] : nil
        }
        for (index, number) in [duration / 3600, duration / 60 % 60, duration % 60].enumerated() {
            guard let element = slider(index) else { throw ClockTimerFailure(step: "find the three timer duration controls") }
            if ClockTimers.durationComponent(attribute(element, kAXValueAttribute)) == number { continue }
            try focus(element)
            do {
                try ClockTimers.adjustDurationComponent(to: number, value: {
                    guard let current = slider(index), attribute(current, kAXSelectedAttribute) as? Bool == true else { return nil }
                    return ClockTimers.durationComponent(attribute(current, kAXValueAttribute))
                }, adjust: { increasing in
                    guard let current = slider(index), AXUIElementPerformAction(current, (increasing ? kAXIncrementAction : kAXDecrementAction) as CFString) == .success else { return false }
                    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
                    return true
                })
            } catch { throw ClockTimerFailure(step: "set timer duration component \(index + 1)") }
        }
        let sheet = find(root, role: kAXSheetRole)
        let scope = sheet.count == 1 ? sheet[0] : root
        let fields = find(scope, role: kAXTextFieldRole).filter { attribute($0, kAXPlaceholderValueAttribute) as? String == "Timer" }
        guard fields.count == 1, AXUIElementSetAttributeValue(fields[0], kAXValueAttribute as CFString, label as CFString) == .success else { throw ClockTimerFailure(step: "set the timer label") }
        try press(find(scope, identifier: "PauseResumeButton").filter { name($0) == "Start" }, step: "start the labeled timer")
        started = true
    }

    func remove(label: String) throws {
        let rows = find(root).filter { element in
            let identifier = attribute(element, kAXIdentifierAttribute) as? String ?? ""
            return identifier.contains("timer") && find(element, role: kAXTextFieldRole).contains(where: { attribute($0, kAXValueAttribute) as? String == label }) && find(element, identifier: "CancelButton").count == 1
        }
        guard let row = rows.last else { throw ClockTimerFailure() }
        try press(find(row, identifier: "CancelButton"))
    }

    private func dismissTimerForm() throws {
        let sheets = find(root, role: kAXSheetRole).filter { find($0, identifier: "TimePicker").count == 1 }
        if sheets.count == 1 { try press(find(sheets[0], identifier: "CancelButton"), step: "close the unused timer form") }
    }

    func removeRecent(label: String) throws {
        try dismissTimerForm()
        let candidates = find(root).filter { (attribute($0, kAXIdentifierAttribute) as? String ?? "").hasPrefix("recent-timer-") }
        let rows = candidates.filter { element in
            find(element).contains { name($0).contains(label) || (attribute($0, kAXValueAttribute) as? String ?? "").contains(label) }
        }
        guard rows.count == 1 else {
            if let data = try? JSONSerialization.data(withJSONObject: ["recent_timer_controls": candidates.count, "matching_fixture_controls": rows.count], options: [.sortedKeys]) {
                FileHandle.standardError.write(data + Data("\n".utf8))
            }
            throw ClockTimerFailure(step: "find the tagged recent timer")
        }
        func deleteItems() -> [AXUIElement] {
            find(root, role: kAXMenuItemRole).filter { name($0) == "Delete" && attribute($0, kAXEnabledAttribute) as? Bool == true }
        }
        _ = AXUIElementSetAttributeValue(rows[0], kAXFocusedAttribute as CFString, kCFBooleanTrue)
        _ = AXUIElementSetAttributeValue(rows[0], kAXSelectedAttribute as CFString, kCFBooleanTrue)
        _ = AXUIElementPerformAction(rows[0], kAXShowMenuAction as CFString)
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        if deleteItems().isEmpty { try click(rows[0], button: .right) }
        if deleteItems().count != 1 {
            let menus = find(root, role: kAXMenuRole)
            let items = menus.flatMap { find($0, role: kAXMenuItemRole) }
            let details: [String: Any] = ["timer_context_menus": menus.count, "timer_context_menu_items": items.count, "timer_context_delete_items": items.filter { name($0) == "Delete" }.count, "timer_context_delete_timer_items": items.filter { name($0) == "Delete Timer" }.count]
            if let data = try? JSONSerialization.data(withJSONObject: details, options: [.sortedKeys]) { FileHandle.standardError.write(data + Data("\n".utf8)) }
        }
        try press(deleteItems(), step: "delete the tagged recent timer")
    }
}
