import AppKit
import ApplicationServices
import SourceSupport

@MainActor
public enum RemindersSectionsUI {
    private static func value(_ element: AXUIElement, _ key: String) -> Any? {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return nil }
        return result
    }
    private static func descendants(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
        guard depth < 30 else { return [] }
        return [element] + ((value(element, kAXChildrenAttribute) as? [AXUIElement]) ?? []).flatMap { descendants($0, depth: depth + 1) }
    }
    private static func root() throws -> AXUIElement {
        guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.reminders").first else { throw SourceFailure.permissionMissing }
        app.activate()
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 1)
        return root
    }
    public static func selectList(_ title: String) throws {
        guard title.hasPrefix("[ab-test]") else { throw SourceFailure.invalid }
        let matches = descendants(try root()).filter { (value($0, kAXTitleAttribute) as? String) == title || (value($0, kAXValueAttribute) as? String) == title }
        guard let target = matches.first else { throw SourceFailure.unavailable }
        if AXUIElementPerformAction(target, kAXPressAction as CFString) != .success {
            guard AXUIElementSetAttributeValue(target, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else { throw SourceFailure.unavailable }
        }
    }
    public static func press(_ title: String) throws {
        let matches = descendants(try root()).filter { (value($0, kAXTitleAttribute) as? String) == title && (value($0, kAXEnabledAttribute) as? Bool) != false }
        guard let target = matches.last, AXUIElementPerformAction(target, kAXPressAction as CFString) == .success else { throw SourceFailure.unavailable }
    }
    public static func fill(_ text: String) throws {
        let matches = descendants(try root()).filter { (value($0, kAXRoleAttribute) as? String) == kAXTextFieldRole && (value($0, kAXFocusedAttribute) as? Bool) == true }
        guard matches.count == 1, AXUIElementSetAttributeValue(matches[0], kAXValueAttribute as CFString, text as CFString) == .success else { throw SourceFailure.unavailable }
    }
    public static func createSection(_ title: String) async throws {
        guard title.hasPrefix("[ab-test]") else { throw SourceFailure.invalid }
        try press("File")
        try press("New Section")
        try await Task.sleep(for: .milliseconds(200))
        try fill(title)
        try key(36)
    }
    public static func move(_ item: String, to section: String) throws {
        guard item.hasPrefix("[ab-test]"), section.hasPrefix("[ab-test]") else { throw SourceFailure.invalid }
        let matches = descendants(try root()).filter { (value($0, kAXValueAttribute) as? String) == item || (value($0, kAXTitleAttribute) as? String) == item }
        guard matches.count == 1 else { throw SourceFailure.conflict }
        guard AXUIElementSetAttributeValue(matches[0], kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success else { throw SourceFailure.unavailable }
        try press("Edit")
        try press("Move to Section")
        try press(section)
    }
    public static func key(_ code: CGKeyCode) throws {
        guard AXIsProcessTrusted(), let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.permissionMissing }
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }
}
