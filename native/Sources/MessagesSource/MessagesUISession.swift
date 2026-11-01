import Foundation
import AppKit
import ApplicationServices
import SourceSupport

public enum MessagesUIInput: Sendable, Equatable {
    case click(CGPoint), controlClick(CGPoint, dismissFirst: Bool), keys([Int], command: Bool), replaceText(String), clearText
    var script: String {
        switch self {
        case .click(let point): MessagesUIScript.click(point)
        case .controlClick(let point, let dismiss): MessagesUIScript.controlClick(point, dismissFirst: dismiss)
        case .keys(let codes, let command): MessagesUIScript.keys(codes, command: command)
        case .replaceText(let text): MessagesUIScript.replaceText(text)
        case .clearText: MessagesUIScript.clearText()
        }
    }
}

public protocol MessagesUIDriver: Sendable {
    func frontmostProcess() -> pid_t?
    func messagesProcess() -> pid_t?
    func activateMessages()
    func systemEvents(_ script: String) -> Int
    func input(_ input: MessagesUIInput) -> Int
}

extension MessagesUIDriver {
    public func input(_ input: MessagesUIInput) -> Int { systemEvents(input.script) }
}

public struct LiveMessagesUIDriver: MessagesUIDriver {
    public init() {}
    public func frontmostProcess() -> pid_t? { NSWorkspace.shared.frontmostApplication?.processIdentifier }
    public func messagesProcess() -> pid_t? { MessagesNavigation.application?.processIdentifier }
    public func activateMessages() {
        if let application = MessagesNavigation.application { _ = application.activate() }
        else if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.MobileSMS") {
            NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration(), completionHandler: nil)
        }
    }
    public func systemEvents(_ script: String) -> Int { MessagesAppleScript.execute(script) }
    public func input(_ input: MessagesUIInput) -> Int {
        guard AXIsProcessTrusted(), let pid = messagesProcess(), frontmostProcess() == pid,
              let source = CGEventSource(stateID: .privateState) else { return -1 }
        func key(_ code: Int, command: Bool = false, text: String? = nil) -> Bool {
            guard frontmostProcess() == pid, let key = CGKeyCode(exactly: code),
                  let down = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: true),
                  let up = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false) else { return false }
            down.flags = command ? .maskCommand : []
            up.flags = command ? .maskCommand : []
            if let text {
                let units = Array(text.utf16)
                units.withUnsafeBufferPointer { buffer in
                    if let address = buffer.baseAddress {
                        down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: address)
                        up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: address)
                    }
                }
            }
            down.postToPid(pid)
            up.postToPid(pid)
            Thread.sleep(forTimeInterval: 0.03)
            return true
        }
        func click(_ point: CGPoint, control: Bool) -> Bool {
            guard frontmostProcess() == pid,
                  let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left),
                  let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left) else { return false }
            down.flags = control ? .maskControl : []
            up.flags = control ? .maskControl : []
            down.postToPid(pid)
            up.postToPid(pid)
            Thread.sleep(forTimeInterval: 0.05)
            return true
        }
        let passed: Bool
        switch input {
        case .click(let point): passed = click(point, control: false)
        case .controlClick(let point, let dismiss): passed = (!dismiss || key(MessagesUISession.escape)) && click(point, control: true)
        case .keys(let codes, let command): passed = codes.allSatisfy { key($0, command: command) }
        case .replaceText(let text): passed = key(0, command: true) && text.allSatisfy { key(0, text: String($0)) }
        case .clearText: passed = key(0, command: true) && key(51)
        }
        return passed ? 0 : -1
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
    public nonisolated static func canReplaceDraft(_ previous: String, with text: String) -> Bool {
        if previous.isEmpty { return true }
        guard let range = text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) else { return false }
        return previous.hasPrefix(String(text[range]))
    }
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
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if let messages = driver.messagesProcess(), driver.frontmostProcess() == messages { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
    }
    public func fence() throws { guard driver.frontmostProcess() == pid else { throw SourceFailure.conflict } }
    public func run(_ script: String) throws {
        try fence()
        guard driver.systemEvents(script) == 0 else { throw SourceFailure.conflict }
    }
    private func input(_ input: MessagesUIInput) throws {
        try fence()
        guard driver.input(input) == 0 else { throw SourceFailure.conflict }
    }
    public func click(at point: CGPoint) throws { try input(.click(point)) }
    public func controlClick(at point: CGPoint, dismissFirst: Bool = true) throws { try input(.controlClick(point, dismissFirst: dismissFirst)) }
    public func press(_ code: Int, command: Bool = false) throws { try input(.keys([code], command: command)) }
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
        try input(.replaceText(text))
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element) == text { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
    public func clear(_ element: AXUIElement, value: (AXUIElement) -> String) throws -> Bool {
        try focus(element, failure: MessagesUIFailure.composerUnavailable)
        try input(.clearText)
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if value(element).isEmpty { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.messageValueRejected
    }
}
