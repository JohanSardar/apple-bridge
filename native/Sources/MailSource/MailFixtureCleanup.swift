#if LAB
import AppKit
import ApplicationServices
import SourceSupport

public enum MailFixtureCleanup {
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
        _ = try await script("""
        repeat with i from (count outgoing messages) to 1 by -1
            set m to outgoing message i
            if subject of m is \(q(tag)) then close m saving no
        end repeat
        return ""
        """)
        for _ in 0..<8 {
            let selected = try await script("""
            set m to missing value
            repeat with b in {inbox, sent mailbox, drafts mailbox}
                set owned to (messages of b whose subject is \(q(tag)) and deleted status is false)
                if (count owned) > 0 then
                    set m to item 1 of owned
                    exit repeat
                end if
            end repeat
            if m is missing value then return "clear"
            if subject of m is not \(q(tag)) then error "Test message selection changed"
            if (count message viewers) is 0 then make new message viewer
            set v to front message viewer
            set selected mailboxes of v to {mailbox of m}
            set selected messages of v to {m}
            set index of window of v to 1
            activate
            set chosen to selected messages of v
            if (count chosen) is not 1 then error "Test message selection is not unique"
            if subject of item 1 of chosen is not \(q(tag)) then error "Test message selection changed"
            return (id of item 1 of chosen) as string
            """)
            if selected == "clear" {
                let outgoing = try await script("return count (outgoing messages whose subject is \(q(tag)))")
                return try MailQualification.cleanupResult("0|" + outgoing)
            }
            guard UInt64(selected) != nil else { throw MailCommandFailure.unconfirmed }
            try await removeSelected()
        }
        throw MailCommandFailure.unconfirmed
    }
}
#endif
