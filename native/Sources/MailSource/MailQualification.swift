#if LAB
import Foundation
import AppKit
import SourceSupport

public struct MailQualification: Sendable {
    public static func deadline(after interval: TimeInterval, action: @escaping @Sendable () -> Void) -> DispatchWorkItem {
        let deadline = DispatchWorkItem(block: action)
        DispatchQueue.global().asyncAfter(deadline: .now() + interval, execute: deadline)
        return deadline
    }
    @MainActor public static func restartUnresponsiveMail() async throws {
        for app in NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail") {
            guard app.forceTerminate() else { throw MailCommandFailure.connectionLost }
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while !NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail").isEmpty {
            guard ContinuousClock.now < deadline else { throw MailCommandFailure.connectionLost }
            try await Task.sleep(for: .milliseconds(100))
        }
    }
    private let restart: @Sendable () async throws -> Void
    private let selectDisplayed: @Sendable (String) async throws -> Void
    private let fixtureIDs: MailFixtureCleanup.FixtureIDs
    private let script: MailCommands.Script
    private let removeSelected: @Sendable () async throws -> Void
    public init(script: @escaping MailCommands.Script = MailCommands.run, fixtureIDs: @escaping MailFixtureCleanup.FixtureIDs = { try MailFixtureCleanup.fixtureIDs(tag: $0) }, selectDisplayed: @escaping @Sendable (String) async throws -> Void = MailFixtureCleanup.selectDisplayed, removeSelected: @escaping @Sendable () async throws -> Void = MailFixtureCleanup.removeSelected, restart: @escaping @Sendable () async throws -> Void = Self.restartUnresponsiveMail) {
        self.selectDisplayed = selectDisplayed
        self.fixtureIDs = fixtureIDs
        self.script = script
        self.removeSelected = removeSelected
        self.restart = restart
    }
    public static func cleanupResult(_ raw: String) throws -> [String: JSON] {
        let counts = raw.split(separator: "|", omittingEmptySubsequences: false)
        guard counts.count == 2, let mailbox = Int(counts[0]), let outgoing = Int(counts[1]), mailbox >= 0, outgoing >= 0 else { throw MailCommandFailure.unconfirmed }
        return ["cleanup_passed": .bool(mailbox == 0 && outgoing == 0), "cleanup_mailbox_clear": .bool(mailbox == 0), "cleanup_outgoing_clear": .bool(outgoing == 0)]
    }
    public func run(scenario: String, tag: String) async throws -> JSON {
        guard tag.range(of: #"^\[ab-test\] [A-Za-z0-9_-]+$"#, options: .regularExpression) != nil,
              ["MAIL-SEND-SELF-REAL", "MAIL-DRAFT-REAL", "MAIL-ORGANIZE-REAL", "MAIL-BODY-FIXTURE", "MAIL-CLEANUP"].contains(scenario) else { throw SourceFailure.invalid }
        let q = MailCommands.quote
        func cleanup() async throws -> [String: JSON] {
            try await MailFixtureCleanup.run(tag: tag, script: script, fixtureIDs: fixtureIDs, selectDisplayed: selectDisplayed, removeSelected: removeSelected)
        }
        if scenario == "MAIL-CLEANUP" {
            do { return .object(try await cleanup()) }
            catch {
                var result: [String: JSON] = ["cleanup_passed": .bool(false), "cleanup_error": .string((error as? MailFixtureCleanup.Failure)?.localizedDescription ?? MailCommands.failureMessage(error))]
                if let failure = ((error as? MailFixtureCleanup.Failure)?.cause ?? error) as? MailCommandFailure, case .script(let code) = failure { result["cleanup_error_number"] = .number(Double(code)) }
                return .object(result)
            }
        }
        var checks: [String: JSON] = [:]
        do {
            do { _ = try await script("return name") }
            catch MailCommandFailure.timeout {
                try await restart()
                _ = try await script("return name")
                checks["unresponsive_mail_recovered"] = .bool(true)
            }
            checks["mail_responsive"] = .bool(true)
            let own = try await script("""
            set m to make new outgoing message with properties {subject:\(q(tag)), visible:true}
            set own to extract address from sender of m
            close m saving no
            return own
            """)
            guard own.contains("@"), !own.contains("\n") else { throw SourceFailure.unavailable }
            checks["self_address_available"] = .bool(true)
            let commands = MailCommands(script: { body in
                guard scenario == "MAIL-BODY-FIXTURE", body.contains("make new outgoing message") else { return try await script(body) }
                return try await script("""
                set priorFormat to default message format
                try
                    set default message format to plain format
                    \(body)
                    set default message format to priorFormat
                on error errorText number errorNumber
                    set default message format to priorFormat
                    error errorText number errorNumber
                end try
                """)
            })
            func apply(_ op: String, _ args: [String: JSON], key: String = "") async throws -> Bool {
                let prepared = try commands.prepare(.object(["action": .object(["op": .string(op), "args": .object(args), "native_key": .string(key)])]))
                guard try await commands.dispatch(prepared)["observed"].bool else { throw MailCommandFailure.unconfirmed }
                return true
            }
            let draft = scenario == "MAIL-DRAFT-REAL"
            let args: [String: JSON] = ["to": .array([.string(own)]), "subject": .string(tag), "body": .string("Apple Bridge plain text fixture " + tag)]
            checks[draft ? "draft_created" : "sent_to_self"] = .bool(try await apply(draft ? "mail.create_draft" : "mail.send", args))
            if !draft {
                var key = ""
                let deadline = ContinuousClock.now.advanced(by: .seconds(30))
                while ContinuousClock.now < deadline {
                    key = try await script("""
                    set found to (messages of inbox whose subject is \(q(tag)))
                    if (count found) is 1 then return (id of item 1 of found) as string
                    return ""
                    """)
                    if UInt64(key) != nil { break }
                    try await Task.sleep(for: .milliseconds(500))
                }
                checks["self_mail_received"] = .bool(UInt64(key) != nil)
                if scenario == "MAIL-ORGANIZE-REAL", UInt64(key) != nil {
                    checks["flag_set"] = .bool(try await apply("mail.set_flag", ["flagged": .bool(true)], key: key))
                    checks["flag_restored"] = .bool(try await apply("mail.set_flag", ["flagged": .bool(false)], key: key))
                }
            }
        } catch {
            checks["fixture_and_commands"] = .bool(false)
            checks["command_error"] = .string(MailCommands.failureMessage(error))
            if let failure = error as? MailCommandFailure, case .script(let code) = failure { checks["command_error_number"] = .number(Double(code)) }
        }
        if scenario != "MAIL-BODY-FIXTURE" {
            do { checks.merge(try await cleanup()) { _, value in value } }
            catch {
                checks["cleanup_passed"] = .bool(false)
                checks["cleanup_error"] = .string((error as? MailFixtureCleanup.Failure)?.localizedDescription ?? MailCommands.failureMessage(error))
                if let failure = ((error as? MailFixtureCleanup.Failure)?.cause ?? error) as? MailCommandFailure, case .script(let code) = failure { checks["cleanup_error_number"] = .number(Double(code)) }
            }
        }
        return .object(checks)
    }
}
#endif
