#if LAB
import AppKit
import ApplicationServices
import SourceSupport

public enum MailFixtureCleanup {
    public struct Failure: Error, LocalizedError, Sendable {
        public let step: String
        public let cause: any Error
        public var errorDescription: String? { "Mail could not " + step + ". Open Mail, let mailbox sync finish, and try again." }
    }
    @MainActor public static func removeSelected() async throws {
        guard AXIsProcessTrusted(), let mail = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail").first else { throw MailCommandFailure.rejected }
        let app = AXUIElementCreateApplication(mail.processIdentifier)
        AXUIElementSetMessagingTimeout(app, 2)
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXMenuBarAttribute as CFString, &value) == .success, let value else { throw MailCommandFailure.rejected }
        func find(_ element: AXUIElement, depth: Int) -> AXUIElement? {
            guard depth < 5 else { return nil }
            var title: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &title)
            if ["Delete", "Move to Trash"].contains(title as? String ?? "") {
                var enabled: CFTypeRef?
                if AXUIElementCopyAttributeValue(element, kAXEnabledAttribute as CFString, &enabled) == .success, enabled as? Bool == true { return element }
            }
            var children: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children) == .success {
                for child in children as? [AXUIElement] ?? [] {
                    if let found = find(child, depth: depth + 1) { return found }
                }
            }
            return nil
        }
        guard let item = find(unsafeDowncast(value, to: AXUIElement.self), depth: 0), AXUIElementPerformAction(item, kAXPressAction as CFString) == .success else { throw MailCommandFailure.rejected }
        try await Task.sleep(for: .milliseconds(400))
    }
    public static func run(tag: String, script: MailCommands.Script, removeSelected: @Sendable () async throws -> Void) async throws -> [String: JSON] {
        let q = MailCommands.quote
        func perform(_ step: String, _ body: String) async throws -> String {
            do { return try await script(body) }
            catch { throw Failure(step: step, cause: error) }
        }
        _ = try await perform("close the tagged compose window", """
        set ownedIDs to id of (outgoing messages whose subject is \(q(tag)))
        repeat with ownedID in ownedIDs
            set m to outgoing message id (contents of ownedID)
            if subject of m is \(q(tag)) then
                set visible of m to true
                close m saving no
            end if
        end repeat
        return ""
        """)
        for _ in 0..<8 {
            let selected = try await perform("select the tagged test message", """
            set cleanupStage to 1
            try
            set m to missing value
            repeat with b in {inbox, sent mailbox, drafts mailbox}
                set owned to (messages of b whose subject is \(q(tag)) and deleted status is false)
                if (count owned) > 0 then
                    set m to item 1 of owned
                    exit repeat
                end if
            end repeat
            set cleanupStage to 2
            if m is missing value then return "clear"
            if subject of m is not \(q(tag)) then error "Test message selection changed"
            if (count message viewers) is 0 then make new message viewer
            set cleanupStage to 3
            set v to front message viewer
            set index of window of v to 1
            activate
            set selected mailboxes of v to {mailbox of m}
            set cleanupStage to 4
            repeat 20 times
                set viewerOwned to (messages of v whose subject is \(q(tag)) and deleted status is false)
                if (count viewerOwned) > 0 then exit repeat
                delay 0.1
            end repeat
            if (count viewerOwned) is 0 then return "failure|9|-2700"
            set selected messages of v to {item 1 of viewerOwned}
            set cleanupStage to 5
            repeat 20 times
                set chosen to selected messages of v
                if (count chosen) is 1 then
                    if subject of item 1 of chosen is \(q(tag)) then return (id of item 1 of chosen) as string
                end if
                delay 0.1
            end repeat
            if (count chosen) is 0 then return "failure|6|-2700"
            if (count chosen) is greater than 1 then return "failure|7|-2700"
            return "failure|8|-2700"
            on error errorText number errorNumber
                return "failure|" & cleanupStage & "|" & errorNumber
            end try
            """)
            if selected == "clear" {
                let outgoing = try await script("return count (outgoing messages whose subject is \(q(tag)))")
                return try MailQualification.cleanupResult("0|" + outgoing)
            }
            if selected.hasPrefix("failure|") {
                let parts = selected.split(separator: "|")
                let steps = ["find the tagged test message", "read the tagged test message", "show the tagged mailbox", "select the tagged test message", "confirm the tagged test selection", "select the test message in the empty viewer", "select only one test message", "confirm the selected test message identity", "find the tagged message in the displayed mailbox"]
                if parts.count == 3, let stage = Int(parts[1]), steps.indices.contains(stage - 1), let code = Int(parts[2]) { throw Failure(step: steps[stage - 1], cause: MailCommandFailure.script(code)) }
            }
            guard UInt64(selected) != nil else { throw MailCommandFailure.unconfirmed }
            try await removeSelected()
        }
        throw MailCommandFailure.unconfirmed
    }
}
#endif
