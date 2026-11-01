#if LAB
import AppKit
import ApplicationServices
import SourceSupport

public enum MailFixtureCleanup {
    public typealias FixtureIDs = @Sendable (String) async throws -> [String]
    public static func fixtureIDs(tag: String, path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Mail/V10/MailData/Envelope Index").path) throws -> [String] {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT 'mail-cleanup'")
        let found = try reader.query("SELECT CAST(m.ROWID AS TEXT) AS id, b.url FROM messages m JOIN subjects s ON s.ROWID=m.subject LEFT JOIN mailboxes b ON b.ROWID=m.mailbox WHERE s.subject=? AND m.deleted=0 ORDER BY m.ROWID", values: [tag], maxRows: 16, timeout: 2)
        guard found.complete else { throw MailCommandFailure.unconfirmed }
        return try found.rows.compactMap { row in
            guard let id = row["id"]?.text, UInt64(id) != nil, let raw = row["url"]?.text, let url = URL(string: raw) else { throw SourceFailure.corrupt }
            if ["trash", "deleted messages", "deleted items", "bin"].contains(url.lastPathComponent.lowercased()) { return nil }
            return id
        }
    }
    public struct Failure: Error, LocalizedError, Sendable {
        public let step: String
        public let cause: any Error
        public var recovery = "Open Mail, let mailbox sync finish, and try again."
        public var errorDescription: String? { "Mail could not " + step + ". " + MailCommands.failureMessage(cause) + " " + recovery }
    }
    @MainActor public static func selectDisplayed(tag: String) async throws {
        guard AXIsProcessTrusted() else { throw Failure(step: "access its Accessibility controls", cause: MailCommandFailure.rejected, recovery: "Enable Apple Bridge in System Settings > Privacy & Security > Accessibility, then try again.") }
        guard let mail = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail").first else { throw MailCommandFailure.connectionLost }
        let app = AXUIElementCreateApplication(mail.processIdentifier)
        AXUIElementSetMessagingTimeout(app, 1)
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        var visited = 0
        func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
            guard ContinuousClock.now < deadline else { return nil }
            var value: CFTypeRef?
            guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
            return value
        }
        func find(_ element: AXUIElement, row: AXUIElement?, depth: Int) -> AXUIElement? {
            visited += 1
            guard depth < 20, visited < 2000, ContinuousClock.now < deadline else { return nil }
            let currentRow = attribute(element, kAXRoleAttribute) as? String == kAXRowRole ? element : row
            if let currentRow {
                for name in [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute] {
                    if attribute(element, name) as? String == tag { return currentRow }
                }
            }
            for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
                if let found = find(child, row: currentRow, depth: depth + 1) { return found }
            }
            return nil
        }
        let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
        for window in windows {
            guard let row = find(window, row: nil, depth: 0) else { continue }
            guard AXUIElementPerformAction(window, kAXRaiseAction as CFString) == .success else { throw Failure(step: "bring the Mail viewer forward", cause: MailCommandFailure.rejected) }
            let selected = AXUIElementSetAttributeValue(row, kAXSelectedAttribute as CFString, kCFBooleanTrue)
            guard selected == .success || AXUIElementPerformAction(row, kAXPressAction as CFString) == .success else { throw Failure(step: "select the tagged row in the Mail viewer", cause: MailCommandFailure.rejected) }
            try await Task.sleep(for: .milliseconds(200))
            return
        }
        throw Failure(step: "find the tagged row in the Mail viewer", cause: MailCommandFailure.rejected)
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
    public static func run(tag: String, script: MailCommands.Script, fixtureIDs: FixtureIDs, selectDisplayed: @Sendable (String) async throws -> Void, removeSelected: @Sendable () async throws -> Void) async throws -> [String: JSON] {
        let q = MailCommands.quote
        func perform(_ step: String, _ body: String) async throws -> String {
            do { return try await script(body) }
            catch { throw Failure(step: step, cause: error) }
        }
        func closeCompose() async throws {
            _ = try await perform("close the tagged compose window", """
            set ownedIDs to id of (outgoing messages whose subject is \(q(tag)))
            repeat with ownedID in ownedIDs
                set m to outgoing message id (contents of ownedID)
                if subject of m is \(q(tag)) and (count (windows whose name contains \(q(tag)) and visible is true)) > 0 then
                    close m saving no
                end if
            end repeat
            return ""
            """)
        }
        try await closeCompose()
        for _ in 0..<8 {
            let ids = try await fixtureIDs(tag)
            guard ids.count <= 16, ids.allSatisfy({ UInt64($0) != nil }) else { throw MailCommandFailure.unconfirmed }
            var selected = try await perform("select the tagged test message", """
            set cleanupStage to 1
            try
            set m to missing value
            repeat with ownedID in {\(ids.joined(separator: ","))}
                repeat with b in {drafts mailbox, inbox, sent mailbox}
                    try
                        set matching to (messages of b whose id is (contents of ownedID))
                        if (count matching) > 0 then
                            set candidate to item 1 of matching
                            if subject of candidate is \(q(tag)) and deleted status of candidate is false then set m to candidate
                        end if
                    on error errorText number errorNumber
                        if errorNumber is not -1728 then error errorText number errorNumber
                    end try
                    if m is not missing value then exit repeat
                end repeat
                if m is not missing value then exit repeat
            end repeat
            set cleanupStage to 2
            if m is missing value then
                if \(ids.count) is 0 then return "clear"
                return "failure|10|-1728"
            end if
            if subject of m is not \(q(tag)) then error "Test message selection changed"
            if (count message viewers) is 0 then make new message viewer
            set cleanupStage to 3
            set v to front message viewer
            activate
            set cleanupStage to 4
            set selected mailboxes of v to {mailbox of m}
            delay 0.2
            return "select|" & ((id of m) as string)
            on error errorText number errorNumber
                return "failure|" & cleanupStage & "|" & errorNumber
            end try
            """)
            if selected == "clear" {
                try await closeCompose()
                if !(try await fixtureIDs(tag)).isEmpty { continue }
                let outgoing = try await script("return count (outgoing messages whose subject is \(q(tag)))")
                let visible = try await script("return count (outgoing messages whose subject is \(q(tag)) and visible is true)")
                let windows = try await script("return count (windows whose name contains \(q(tag)) and visible is true)")
                guard let retained = Int(outgoing), retained >= 0, let open = Int(visible), open >= 0, let windowCount = Int(windows), windowCount >= 0 else { throw MailCommandFailure.unconfirmed }
                let closed = windowCount == 0
                var result = try MailQualification.cleanupResult("0|" + visible)
                result["cleanup_outgoing_clear"] = .bool(closed)
                result["cleanup_windows_clear"] = .bool(windowCount == 0)
                result["cleanup_passed"] = .bool(closed)
                if retained > 0 { result["cleanup_closed_compose_cache_verified"] = .bool(closed) }
                return result
            }
            if selected.hasPrefix("select|") {
                guard ids.contains(String(selected.dropFirst(7))) else { throw MailCommandFailure.unconfirmed }
                try await selectDisplayed(tag)
                selected = try await perform("confirm the test row selected through Accessibility", """
                repeat 20 times
                    set chosen to selected messages of front message viewer
                    if (count chosen) is 1 then
                        if subject of item 1 of chosen is \(q(tag)) then return (id of item 1 of chosen) as string
                    end if
                    delay 0.1
                end repeat
                return ""
                """)
            }
            if selected.hasPrefix("failure|") {
                let parts = selected.split(separator: "|")
                let steps = [1: "find the tagged test message", 2: "read the tagged test message", 3: "show the tagged mailbox", 4: "choose the tagged mailbox", 10: "locate the indexed test message in Mail"]
                if parts.count == 3, let stage = Int(parts[1]), let step = steps[stage], let code = Int(parts[2]) { throw Failure(step: step, cause: MailCommandFailure.script(code)) }
            }
            guard UInt64(selected) != nil, ids.contains(selected) else { throw MailCommandFailure.unconfirmed }
            try await removeSelected()
        }
        throw MailCommandFailure.unconfirmed
    }
}
#endif
