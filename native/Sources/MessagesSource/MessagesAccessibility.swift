import Foundation
import AppKit
import ApplicationServices
import SourceSupport

public enum MessagesAction: String, Sendable {
    case send, reply, react, edit, unsend, markRead = "mark_read", markUnread = "mark_unread", deleteMessage = "delete_message", deleteChat = "delete_chat", createGroup = "create_group", renameGroup = "rename_group", addMember = "add_member", removeMember = "remove_member", leaveGroup = "leave_group"
    public init(operation: String) throws {
        guard let value = Self(rawValue: String(operation.dropFirst("messages.".count))) else { throw SourceFailure.invalid }
        self = value
    }
}

@UIActor
enum MessagesAccessibility {
    private static let eventMarker: Int64 = 0x41425249444745
    static var available: Bool {
        guard AXIsProcessTrusted(), let session = CGSessionCopyCurrentDictionary() as? [String: Any], session["CGSSessionScreenIsLocked"] as? Bool != true else { return false }
        return true
    }
    private static func value(_ element: AXUIElement, _ attribute: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &result) == .success else { return "" }
        return result as? String ?? ""
    }
    private static func children(_ element: AXUIElement) -> [AXUIElement] {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &result) == .success else { return [] }
        return result as? [AXUIElement] ?? []
    }
    private static func matching(_ root: AXUIElement, budget: inout Int, predicate: (AXUIElement) -> Bool) -> [AXUIElement] {
        guard budget > 0 else { return [] }
        budget -= 1
        var result = predicate(root) ? [root] : []
        for child in children(root) { result += matching(child, budget: &budget, predicate: predicate) }
        return result
    }
    private static func root() throws -> (AXUIElement, pid_t) {
        guard available, let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").first, NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.25)
        return (root, app.processIdentifier)
    }
    private static func target(_ root: AXUIElement, prepared: Prepared) throws -> AXUIElement {
        let guid = prepared.action["native_key"].string
        guard !guid.isEmpty else { throw SourceFailure.capabilityUnproven }
        var budget = 4000
        let targets = matching(root, budget: &budget) { element in
            let identifier = value(element, kAXIdentifierAttribute)
            guard identifier == guid || identifier.hasSuffix("/" + guid) else { return false }
            let expected = prepared.nativeState["target"]["text"].string
            if expected.isEmpty { return true }
            let text = [value(element, kAXValueAttribute), value(element, kAXDescriptionAttribute), value(element, kAXTitleAttribute)].joined(separator: "\n")
            return text.contains(expected)
        }
        guard targets.count == 1, budget > 0 else { throw SourceFailure.conflict }
        return targets[0]
    }
    private static func focus(_ root: AXUIElement) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &result) == .success, let result else { return "" }
        guard CFGetTypeID(result)==AXUIElementGetTypeID() else {return ""}
        let element = unsafeDowncast(result, to: AXUIElement.self)
        return value(element, kAXIdentifierAttribute) + "|" + value(element, kAXRoleAttribute)
    }
    private static func inputAge() -> Double {
        min(CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .keyDown), CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .leftMouseDown))
    }
    static func validate(_ prepared: Prepared) throws {
        let (root, _) = try root()
        _ = try target(root, prepared: prepared)
    }
    private static func press(_ root: AXUIElement, title: String) throws {
        var budget = 4000
        let matches = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXMenuItemRole && value($0, kAXTitleAttribute) == title }
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
    }
    private static func replace(_ root: AXUIElement, text: String) throws {
        var budget = 4000
        let editors = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXTextAreaRole && value($0, kAXIdentifierAttribute) == "message-entry" }
        guard editors.count == 1, AXUIElementSetAttributeValue(editors[0], kAXValueAttribute as CFString, text as CFTypeRef) == .success else { throw SourceFailure.conflict }
    }
    private static func enter(_ pid: pid_t) throws {
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 36, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: 36, keyDown: false) else { throw SourceFailure.unavailable }
        down.setIntegerValueField(.eventSourceUserData, value: eventMarker)
        up.setIntegerValueField(.eventSourceUserData, value: eventMarker)
        down.postToPid(pid)
        up.postToPid(pid)
    }
    private static func screenshot(_ directory: URL) throws -> String {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let path = directory.appendingPathComponent(UUID().uuidString + ".png").path
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        process.arguments = ["-x", path]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw SourceFailure.unavailable }
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
        return path
    }
    static func perform(_ prepared: Prepared) throws -> JSON {
        let action = try MessagesAction(operation: prepared.action["op"].string)
        let args = prepared.action["messages"]
        let (root, pid) = try root()
        let bubble = try target(root, prepared: prepared)
        let initialFocus = focus(root)
        let initialAge = inputAge()
        let start = ContinuousClock.now
        let directory = PrivateReport.directory.appendingPathComponent("messages-ui-evidence", isDirectory: true)
        let before = try screenshot(directory)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, focus(root) == initialFocus, inputAge() >= initialAge else { throw SourceFailure.conflict }
        guard AXUIElementPerformAction(bubble, kAXShowMenuAction as CFString) == .success else { throw SourceFailure.conflict }
        let menu: String
        switch action {
        case .reply: menu = "Reply"
        case .react: menu = "Tapback"
        case .edit: menu = "Edit"
        case .unsend: menu = "Undo Send"
        case .markRead: menu = "Mark as Read"
        case .markUnread: menu = "Mark as Unread"
        case .deleteMessage, .deleteChat: menu = "Delete"
        case .renameGroup: menu = "Change Group Name"
        case .addMember: menu = "Add Member"
        case .removeMember: menu = "Remove from Conversation"
        case .leaveGroup: menu = "Leave this Conversation"
        default: throw SourceFailure.capabilityUnproven
        }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, inputAge() >= initialAge else { throw SourceFailure.conflict }
        try press(root, title: menu)
        switch action {
        case .reply, .edit:
            try replace(root, text: args["text"].string)
            guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { throw SourceFailure.conflict }
            try enter(pid)
        case .react:
            let labels = ["love": "Love", "like": "Like", "dislike": "Dislike", "laugh": "Laugh", "emphasize": "Emphasize", "question": "Question"]
            if args["reaction"].string == "emoji" { throw SourceFailure.capabilityUnproven }
            guard let label = labels[args["reaction"].string] else { throw SourceFailure.invalid }
            try press(root, title: label)
        case .deleteMessage, .deleteChat: try press(root, title: "Delete")
        case .renameGroup: try replace(root, text: args["name"].string); try enter(pid)
        case .addMember: try replace(root, text: args["handle"].string); try enter(pid)
        default: break
        }
        let after = try screenshot(directory)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, start.duration(to: ContinuousClock.now) < .seconds(10) else { throw SourceFailure.conflict }
        return .object(["before_screenshot": .string(before), "after_screenshot": .string(after), "target_verified": .bool(true)])
    }
}
