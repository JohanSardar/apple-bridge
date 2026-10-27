import Foundation
import MessagesSource
import SourceSupport

final class ScriptedUIDriver: MessagesUIDriver, @unchecked Sendable {
    private let lock = NSLock()
    private var front: [pid_t?]
    private var recorded: [String] = []
    private var activated = 0
    private let messages: pid_t
    init(messages: pid_t, front: [pid_t?]) {
        self.messages = messages
        self.front = front
    }
    var scripts: [String] { lock.withLock { recorded } }
    var activations: Int { lock.withLock { activated } }
    func frontmostProcess() -> pid_t? { lock.withLock { front.count > 1 ? front.removeFirst() : front.first ?? nil } }
    func messagesProcess() -> pid_t? { messages }
    func activateMessages() { lock.withLock { activated += 1 } }
    func systemEvents(_ script: String) -> Int { lock.withLock { recorded.append(script) }; return 0 }
}

struct UISessionChecks {
    @UIActor
    static func run() throws {
        let messages: pid_t = 4242, other: pid_t = 999
        var refused = 0
        for (driver, trusted) in [(ScriptedUIDriver(messages: messages, front: [other]), true), (ScriptedUIDriver(messages: messages, front: [nil]), true), (ScriptedUIDriver(messages: messages, front: [messages]), false)] {
            do { _ = try MessagesUISession.frontmost(driver, trusted: trusted) } catch SourceFailure.conflict { refused += 1 }
        }
        try MessagesCaptureChecks.require(refused == 3, "session refuses a Messages app that is not frontmost and trusted")
        let steady = ScriptedUIDriver(messages: messages, front: [messages])
        let session = try MessagesUISession.frontmost(steady, trusted: true)
        try session.click(at: CGPoint(x: 10.6, y: 20.2))
        try session.controlClick(at: CGPoint(x: 1, y: 2))
        try session.press(MessagesUISession.enter)
        try session.press(0, command: true)
        let scripts = steady.scripts
        try MessagesCaptureChecks.require(scripts.count == 4 && scripts.allSatisfy { $0.hasPrefix("tell application \"System Events\"\ntell process \"Messages\"\n") && $0.hasSuffix("\nend tell\nend tell") }, "every step runs one System Events script")
        try MessagesCaptureChecks.require(scripts[0].contains("key up control\nclick at {10,20}") && scripts[2].contains("key up control\nkey code 36") && scripts[3].contains("key code 0 using {command down}"), "click and key scripts release control first")
        let menu = scripts[1].components(separatedBy: "\n")
        try MessagesCaptureChecks.require(menu.contains("key code 53") && menu.firstIndex(of: "key down control")! < menu.firstIndex(of: "click at {1,2}")! && menu.filter { $0 == "key up control" }.count == 2 && menu[menu.count - 3] == "key up control" && menu.contains("error number -1"), "control click always releases control")
        let interrupted = ScriptedUIDriver(messages: messages, front: [messages, messages, other])
        let fenced = try MessagesUISession.frontmost(interrupted, trusted: true)
        try fenced.press(MessagesUISession.escape)
        var aborted = false
        do { try fenced.press(MessagesUISession.enter) } catch SourceFailure.conflict { aborted = true }
        try MessagesCaptureChecks.require(aborted && interrupted.scripts.count == 1 && !interrupted.scripts[0].contains("key code 36"), "interference aborts before the next step runs")
        MessagesUISession.activate(steady, settle: 0)
        try MessagesCaptureChecks.require(steady.activations == 1, "activation goes through the driver")
        try MessagesCaptureChecks.require(MessagesUIScript.replaceText("say \"hi\"").contains("characters of " + MessagesAppleScript.literal("say \"hi\"")) && MessagesUIScript.controlClick(CGPoint(x: 5, y: 6), dismissFirst: false).components(separatedBy: "\n").contains("key code 53") == false, "typed text is quoted and dismiss is optional")
        print("all Messages UI session checks passed")
    }
}
