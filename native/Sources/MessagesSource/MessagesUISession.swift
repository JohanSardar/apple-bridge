import Foundation
import AppKit
import ApplicationServices
import SourceSupport

public protocol MessagesUIDriver: Sendable {
    func frontmostProcess() -> pid_t?
    func messagesProcess() -> pid_t?
    func activateMessages()
    func systemEvents(_ script: String) -> Int
}

public struct LiveMessagesUIDriver: MessagesUIDriver {
    public init() {}
    public func frontmostProcess() -> pid_t? { NSWorkspace.shared.frontmostApplication?.processIdentifier }
    public func messagesProcess() -> pid_t? { MessagesNavigation.application?.processIdentifier }
    public func activateMessages() { _ = MessagesNavigation.application?.activate() }
    public func systemEvents(_ script: String) -> Int { MessagesAppleScript.execute(script) }
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
    public static let enter = 36
    public static let escape = 53
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
        guard trusted, let pid = driver.messagesProcess(), driver.frontmostProcess() == pid else {
            try? PrivateReport.write(["stage": "frontmost_application"], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-prepare-diagnostic.json").path))
            throw SourceFailure.conflict
        }
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
    public func click(at point: CGPoint) throws { try run(MessagesUIScript.click(point)) }
    public func controlClick(at point: CGPoint, dismissFirst: Bool = true) throws { try run(MessagesUIScript.controlClick(point, dismissFirst: dismissFirst)) }
    public func press(_ code: Int, command: Bool = false) throws { try run(MessagesUIScript.keys([code], command: command)) }
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
        try run(MessagesUIScript.replaceText(text))
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element) == text { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
    public func clear(_ element: AXUIElement, value: (AXUIElement) -> String) throws -> Bool {
        try focus(element, failure: MessagesUIFailure.composerUnavailable)
        try run(MessagesUIScript.clearText())
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element).isEmpty { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
}
