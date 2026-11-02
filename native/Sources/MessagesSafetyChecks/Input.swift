import Foundation
import MessagesSource
import SourceSupport

final class AccessibilityInputDriver: MessagesUIDriver, @unchecked Sendable {
    private let lock = NSLock()
    private var events: [MessagesUIInput] = []
    private var automation = 0
    private var front: [pid_t]
    init(front: [pid_t] = [4242]) { self.front = front }
    var inputs: [MessagesUIInput] { lock.withLock { events } }
    var automationCalls: Int { lock.withLock { automation } }
    func frontmostProcess() -> pid_t? { lock.withLock { front.count > 1 ? front.removeFirst() : front.first } }
    func messagesProcess() -> pid_t? { 4242 }
    func activateMessages() {}
    func systemEvents(_ script: String) -> Int { lock.withLock { automation += 1 }; return -1 }
    func input(_ input: MessagesUIInput) -> Int { lock.withLock { events.append(input) }; return 0 }
}

enum AccessibilityInputChecks {
    @UIActor
    static func run() throws -> [String: Bool] {
        let driver = AccessibilityInputDriver()
        let session = try MessagesUISession.frontmost(driver, trusted: true)
        try session.click(at: CGPoint(x: 1, y: 2))
        try session.controlClick(at: CGPoint(x: 3, y: 4))
        try session.press(MessagesUISession.enter)
        try session.press(0, command: true)
        let expected: [MessagesUIInput] = [.click(CGPoint(x: 1, y: 2)), .controlClick(CGPoint(x: 3, y: 4), dismissFirst: true), .keys([36], command: false), .keys([0], command: true)]
        let interrupted = AccessibilityInputDriver(front: [4242, 4242, 999])
        let guarded = try MessagesUISession.frontmost(interrupted, trusted: true)
        try guarded.press(MessagesUISession.escape)
        var refused = false
        do { try guarded.press(MessagesUISession.enter) } catch SourceFailure.conflict { refused = true }
        let delayed = AccessibilityInputDriver(front: [999, 999, 4242])
        MessagesUISession.activate(delayed, settle: 0)
        let ready = (try? MessagesUISession.frontmost(delayed, trusted: true)) != nil
        return ["accessibility_input_without_automation": driver.inputs == expected && driver.automationCalls == 0,
                "activation_waits_for_foreground_readiness": ready && delayed.inputs.isEmpty,
                "accessibility_input_interference_refused": refused && interrupted.inputs == [.keys([53], command: false)] && interrupted.automationCalls == 0]
    }
}
