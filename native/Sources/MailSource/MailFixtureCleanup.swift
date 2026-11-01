#if LAB
import AppKit
import ApplicationServices
import SourceSupport

public enum MailFixtureCleanup {
    public typealias FixtureIDs = @Sendable (String) async throws -> [String]
    public static func fixtureIDs(tag: String, path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Mail/V10/MailData/Envelope Index").path) throws -> [String] {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT 'mail-cleanup'")
        let found = try reader.query("SELECT trim(g.message_id_header, '<>') AS id, b.url FROM messages m JOIN subjects s ON s.ROWID=m.subject LEFT JOIN message_global_data g ON g.message_id=m.message_id LEFT JOIN mailboxes b ON b.ROWID=m.mailbox WHERE s.subject=? AND m.deleted=0 ORDER BY m.ROWID", values: [tag], maxRows: 16, timeout: 2)
        guard found.complete else { throw MailCommandFailure.unconfirmed }
        return try found.rows.compactMap { row in
            guard let id = row["id"]?.text, !id.isEmpty && id.contains("@"), let raw = row["url"]?.text, let url = URL(string: raw) else { throw SourceFailure.corrupt }
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
        guard !tag.isEmpty, !tag.contains("\n"), !tag.contains("\r"), tag.contains("@") else { throw MailCommandFailure.unconfirmed }
        let opened = try await MailCommands.run("""
        repeat with b in {drafts mailbox, inbox, sent mailbox}
            set found to (messages of b whose message id is \(MailCommands.quote(tag)) and deleted status is false)
            if (count found) > 0 then
                open item 1 of found
                activate
                return "opened"
            end if
        end repeat
        return ""
        """)
        guard opened == "opened" else { throw MailCommandFailure.unconfirmed }
        try await Task.sleep(for: .milliseconds(500))
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
            if (count (windows whose name contains \(q(tag)) and visible is true)) is 0 then return ""
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
        for attempt in 0..<40 {
            let ids = try await fixtureIDs(tag)
            guard ids.count <= 16, ids.allSatisfy({ !$0.isEmpty && $0.contains("@") && !$0.contains("\n") && !$0.contains("\r") }) else { throw MailCommandFailure.unconfirmed }
            var selected = try await perform("select the tagged test message", """
            set cleanupStage to 1
            try
            set m to missing value
            repeat with ownedID in {\(ids.map(q).joined(separator: ","))}
                repeat with b in {drafts mailbox, inbox, sent mailbox}
                    try
                        set matching to (messages of b whose message id is (contents of ownedID) and deleted status is false)
                        if (count matching) > 0 then
                            set candidate to item 1 of matching
                            if subject of candidate is \(q(tag)) and deleted status of candidate is false then
                                set own to extract address from sender of candidate
                                set ownRecipients to address of every to recipient of candidate
                                if own is in ownRecipients then set m to candidate
                            end if
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
            set cleanupStage to 3
            set headerID to message id of m
            if headerID is "" then return "failure|3|-2700"
            return "select|" & headerID
            on error errorText number errorNumber
                return "failure|" & cleanupStage & "|" & errorNumber
            end try
            """)
            if selected == "clear" {
                try await closeCompose()
                if !(try await fixtureIDs(tag)).isEmpty { continue }
                let windows = try await script("return count (windows whose name contains \(q(tag)) and visible is true)")
                guard let windowCount = Int(windows), windowCount >= 0 else { throw MailCommandFailure.unconfirmed }
                let closed = windowCount == 0
                var result = try MailQualification.cleanupResult("0|" + windows)
                result["cleanup_outgoing_clear"] = .bool(closed)
                result["cleanup_windows_clear"] = .bool(windowCount == 0)
                result["cleanup_passed"] = .bool(closed)
                return result
            }
            if selected.hasPrefix("select|") {
                let header = String(selected.dropFirst(7))
                guard ids.contains(header) else { throw MailCommandFailure.unconfirmed }
                try await selectDisplayed(header)
                selected = try await perform("confirm the test message opened by Message-ID", """
                repeat 20 times
                    if (count windows) > 0 then
                        if name of front window contains \(q(tag)) then return "opened"
                    end if
                    delay 0.1
                end repeat
                return ""
                """)
            }
            if selected.hasPrefix("failure|") {
                let parts = selected.split(separator: "|")
                let steps = [1: "find the tagged test message", 2: "read the tagged test message", 3: "show the tagged mailbox", 4: "choose the tagged mailbox", 10: "locate the indexed test message in Mail"]
                if parts.count == 3, let stage = Int(parts[1]), let step = steps[stage], let code = Int(parts[2]) {
                    if attempt < 39, [4, 10].contains(stage), [-1728, -1700].contains(code) {
                        try await Task.sleep(for: .milliseconds(500))
                        continue
                    }
                    throw Failure(step: step, cause: MailCommandFailure.script(code))
                }
            }
            guard selected == "opened" else { throw MailCommandFailure.unconfirmed }
            try await removeSelected()
            let settle = ContinuousClock.now.advanced(by: .seconds(8))
            while try await fixtureIDs(tag) == ids, ContinuousClock.now < settle {
                try await Task.sleep(for: .milliseconds(500))
            }
        }
        throw MailCommandFailure.unconfirmed
    }
}
#endif
