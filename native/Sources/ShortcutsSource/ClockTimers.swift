import Foundation
import AppKit
import ApplicationServices
import SQLite3
import SourceSupport

@MainActor
public enum ClockTimers {
    private static func failure() -> any Error { ClockTimerFailure() }

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
        case "start_timer":
            guard let label = value["label"] as? String, label.hasPrefix("[ab-test]"), label.utf8.count <= 256, let duration = value["duration"] as? Int, (1...86399).contains(duration), !before.contains(where: { $0["label"] as? String == label }) else { throw failure() }
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
                guard existing["label"] as? String == label else { throw failure() }
                try ClockTimerUI().remove(label: label)
            }
            result["timers"] = try awaitSnapshot { !$0.contains(where: { $0["id"] as? String == identifier }) }
        default: throw failure()
        }
        if result["timers"] == nil { result["timers"] = try snapshot() }
        let output = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        guard output.count <= 32768 else { throw failure() }
        return output
    }

    private static func awaitSnapshot(_ accepts: ([[String: Any]]) -> Bool) throws -> [[String: Any]] {
        let deadline = Date().addingTimeInterval(10)
        repeat {
            let rows = try snapshot()
            if accepts(rows) { return rows }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw failure()
    }

    public static func snapshot(at location: URL? = nil) throws -> [[String: Any]] {
        let path = (location ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.mobiletimerd/local.sqlite")).path
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if let database { sqlite3_close(database) }; throw failure() }
        defer { sqlite3_close(database) }
        sqlite3_busy_timeout(database, 1000)
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT hex(ZMTID), COALESCE(ZTITLE, ''), ZDURATION, ZSTATE FROM ZMTCDTIMER WHERE ZSTATE IN (2, 3) ORDER BY Z_PK", -1, &statement, nil) == SQLITE_OK else { throw failure() }
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
        @MainActor func visit(_ item: AXUIElement, depth: Int) -> [AXUIElement] {
            guard depth < 20, budget > 0 else { return [] }
            budget -= 1
            var result: [AXUIElement] = []
            let actualRole = attribute(item, kAXRoleAttribute) as? String
            let actualIdentifier = attribute(item, kAXIdentifierAttribute) as? String
            if (role == nil || actualRole == role) && (identifier == nil || actualIdentifier == identifier) { result.append(item) }
            for child in attribute(item, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += visit(child, depth: depth + 1) }
            return result
        }
        return visit(element, depth: 0)
    }

    private func press(_ elements: [AXUIElement], step: String = "press the timer control") throws {
        guard elements.count == 1, AXUIElementPerformAction(elements[0], kAXPressAction as CFString) == .success else { throw ClockTimerFailure(step: step) }
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }

    func create(label: String, duration: Int) throws {
        var started = false
        defer {
            if !started, find(root, role: kAXSheetRole).count == 1, NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier {
                for down in [true, false] { CGEvent(keyboardEventSource: nil, virtualKey: 53, keyDown: down)?.postToPid(app.processIdentifier) }
            }
        }
        let add = find(root, role: kAXMenuButtonRole).filter { name($0) == "Add" || name($0) == "Add a timer" }
        if add.count == 1 {
            try press(add, step: "open the timer menu")
            try press(find(root, identifier: "addTimer:"), step: "choose a new timer")
        }
        let picker = find(root, identifier: "TimePicker")
        guard picker.count == 1 else { throw ClockTimerFailure(step: "find the timer duration picker") }
        let sliders = find(picker[0], role: kAXSliderRole)
        guard sliders.count == 3 else { throw ClockTimerFailure(step: "find the three timer duration controls") }
        for (element, number) in zip(sliders, [duration / 3600, duration / 60 % 60, duration % 60]) {
            try ClockTimers.adjustDurationComponent(to: number, value: {
                if let value = attribute(element, kAXValueAttribute) as? NSNumber { return value.intValue }
                if let value = attribute(element, kAXValueAttribute) as? String { return Int(value) }
                return nil
            }, adjust: { increasing in
                let action = increasing ? kAXIncrementAction : kAXDecrementAction
                guard AXUIElementPerformAction(element, action as CFString) == .success else { return false }
                RunLoop.current.run(until: Date().addingTimeInterval(0.1))
                return true
            })
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
}
