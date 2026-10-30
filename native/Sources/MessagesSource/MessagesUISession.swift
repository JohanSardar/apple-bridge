import Foundation
import AppKit
import ApplicationServices
import SourceSupport

public protocol MessagesUIDriver: Sendable {
    func frontmostProcess() -> pid_t?
    func messagesProcess() -> pid_t?
    func activateMessages()
    func systemEvents(_ script: String) -> Int
    func keys(_ codes: [Int], command: Bool) -> Int
    func click(_ point: CGPoint, control: Bool, dismissFirst: Bool) -> Int
    func replace(_ text: String) -> Int
}

public extension MessagesUIDriver {
    func keys(_ codes: [Int], command: Bool) -> Int { systemEvents(MessagesUIScript.keys(codes, command: command)) }
    func click(_ point: CGPoint, control: Bool, dismissFirst: Bool) -> Int { systemEvents(control ? MessagesUIScript.controlClick(point, dismissFirst: dismissFirst) : MessagesUIScript.click(point)) }
    func replace(_ text: String) -> Int { systemEvents(MessagesUIScript.replaceText(text)) }
}

public struct LiveMessagesUIDriver: MessagesUIDriver {
    public init() {}
    public func frontmostProcess() -> pid_t? { NSWorkspace.shared.frontmostApplication?.processIdentifier }
    public func messagesProcess() -> pid_t? { MessagesNavigation.application?.processIdentifier }
    public func activateMessages() { _ = MessagesNavigation.application?.activate() }
    public func systemEvents(_ script: String) -> Int { -1 }
    public func keys(_ codes: [Int], command: Bool) -> Int {
        guard frontmostProcess() == messagesProcess(), messagesProcess() != nil else { return -1 }
        for code in codes {
            guard let value = CGKeyCode(exactly: code), let down = CGEvent(keyboardEventSource: nil, virtualKey: value, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: value, keyDown: false) else { return -1 }
            down.flags = command ? .maskCommand : []
            up.flags = command ? .maskCommand : []
            down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
        }
        return 0
    }
    public func click(_ point: CGPoint, control: Bool, dismissFirst: Bool) -> Int {
        guard frontmostProcess() == messagesProcess(), messagesProcess() != nil else { return -1 }
        if dismissFirst, keys([MessagesUISession.escape], command: false) != 0 { return -1 }
        guard let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left), let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left) else { return -1 }
        down.flags = control ? .maskControl : []
        up.flags = []
        down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
        return 0
    }
    public func replace(_ text: String) -> Int {
        guard keys([0], command: true) == 0 else { return -1 }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        guard keys([51], command: false) == 0 else { return -1 }
        for character in text {
            guard frontmostProcess() == messagesProcess(), let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false) else { return -1 }
            let units = Array(String(character).utf16)
            units.withUnsafeBufferPointer { buffer in
                down.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: buffer.baseAddress!)
                up.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: buffer.baseAddress!)
            }
            down.flags = []; up.flags = []
            down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
        }
        return 0
    }
}

public enum MessagesUIScript {
    static func process(_ lines: [String]) -> String { (["tell application \"System Events\"", "tell process \"Messages\""] + lines + ["end tell", "end tell"]).joined(separator: "\n") }
    static func point(_ point: CGPoint) -> String { "{" + String(Int(point.x)) + "," + String(Int(point.y)) + "}" }
    public static func click(_ at: CGPoint) -> String { process(["set frontmost to true", "key up control", "click at " + point(at)]) }
    public static func controlClick(_ at: CGPoint, dismissFirst: Bool) -> String { process(["set frontmost to true"] + (dismissFirst ? ["key code 53"] : []) + ["key down control", "try", "click at " + point(at), "on error", "key up control", "error number -1", "end try", "key up control"]) }
    public static func keys(_ codes: [Int], command: Bool = false) -> String { process(["key up control"] + codes.map { "key code " + String($0) + (command ? " using {command down}" : "") }) }
    public static func replaceText(_ text: String) -> String { process(["key up control", "key code 0 using {command down}", "delay 0.1", "repeat with characterValue in characters of " + MessagesAppleScript.literal(text), "keystroke (characterValue as text)", "delay 0.015", "end repeat"]) }
    public static func clearText() -> String { process(["key up control", "key code 0 using {command down}", "key code 51"]) }
}

@UIActor
public struct MessagesUISession {
    public nonisolated static let enter = 36
    public nonisolated static let escape = 53
    public let pid: pid_t
    public let root: AXUIElement
    let driver: any MessagesUIDriver
    public init(pid: pid_t, driver: any MessagesUIDriver) {
        self.pid = pid
        self.driver = driver
        root = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(root, 0.25)
    }
    public static func frontmost(_ driver: any MessagesUIDriver = LiveMessagesUIDriver(), trusted: Bool = AXIsProcessTrusted()) throws -> MessagesUISession {
        guard trusted, let pid = driver.messagesProcess(), driver.frontmostProcess() == pid else { throw SourceFailure.conflict }
        return MessagesUISession(pid: pid, driver: driver)
    }
    public static func activate(_ driver: any MessagesUIDriver = LiveMessagesUIDriver(), settle: TimeInterval = 0.2) {
        driver.activateMessages()
        RunLoop.current.run(until: Date().addingTimeInterval(settle))
    }
    public func fence() throws { guard driver.frontmostProcess() == pid else { throw SourceFailure.conflict } }
    public func run(_ script: String) throws {
        try fence()
        guard driver.systemEvents(script) == 0 else { throw SourceFailure.conflict }
    }
    public func click(at point: CGPoint) throws { try fence(); guard driver.click(point, control: false, dismissFirst: false) == 0 else { throw SourceFailure.conflict } }
    public func controlClick(at point: CGPoint, dismissFirst: Bool = true) throws { try fence(); guard driver.click(point, control: true, dismissFirst: dismissFirst) == 0 else { throw SourceFailure.conflict } }
    public func press(_ code: Int, command: Bool = false) throws { try fence(); guard driver.keys([code], command: command) == 0 else { throw SourceFailure.conflict } }
    public func focused(_ element: AXUIElement) -> Bool {
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused else { return false }
        return CFEqual(focused, element)
    }
    public func focus(_ element: AXUIElement, failure: any Error) throws {
        try fence()
        guard AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, focused(element) else { throw failure }
    }
    public func type(_ text: String, into element: AXUIElement, value: (AXUIElement) -> String) throws {
        try focus(element, failure: MessagesUIFailure.composerUnavailable)
        try fence()
        guard driver.replace(text) == 0 else { throw SourceFailure.conflict }
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element) == text { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
    public func clear(_ element: AXUIElement, value: (AXUIElement) -> String) throws -> Bool {
        try focus(element, failure: MessagesUIFailure.composerUnavailable)
        try press(0, command: true)
        try press(51)
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element).isEmpty { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
}
