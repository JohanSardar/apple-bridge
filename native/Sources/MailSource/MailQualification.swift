#if LAB
import Foundation
import SourceSupport

public struct MailQualification: Sendable {
    public static func deadline(after interval: TimeInterval, action: @escaping @Sendable () -> Void) -> DispatchWorkItem {
        let deadline = DispatchWorkItem(block: action)
        DispatchQueue.global().asyncAfter(deadline: .now() + interval, execute: deadline)
        return deadline
    }
    private let script: MailCommands.Script
    public init(script: @escaping MailCommands.Script = MailCommands.run) { self.script = script }
    private func cleanupResult(_ raw: String) throws -> [String: JSON] {
        let counts = raw.split(separator: "|", omittingEmptySubsequences: false)
        guard counts.count == 2, let mailbox = Int(counts[0]), let outgoing = Int(counts[1]), mailbox >= 0, outgoing >= 0 else { throw MailCommandFailure.unconfirmed }
        return ["cleanup_passed": .bool(mailbox == 0 && outgoing == 0), "cleanup_mailbox_clear": .bool(mailbox == 0), "cleanup_outgoing_clear": .bool(outgoing == 0)]
    }
    public func run(scenario: String, tag: String) async throws -> JSON {
        guard tag.range(of: #"^\[ab-test\] [A-Za-z0-9_-]+$"#, options: .regularExpression) != nil,
              ["MAIL-SEND-SELF-REAL", "MAIL-DRAFT-REAL", "MAIL-ORGANIZE-REAL", "MAIL-BODY-FIXTURE", "MAIL-CLEANUP"].contains(scenario) else { throw SourceFailure.invalid }
        let q = MailCommands.quote
        let cleanup = """
        repeat with i from (count outgoing messages) to 1 by -1
            set m to outgoing message i
            if subject of m is \(q(tag)) then close m saving no
        end repeat
        repeat 2 times
            set trashed to id of (messages of trash mailbox whose subject is \(q(tag)))
            repeat with a in accounts
                repeat with b in mailboxes of a
                    set owned to (every message of b whose subject is \(q(tag)))
                    repeat with m in owned
                        if subject of m is \(q(tag)) and (id of m) is not in trashed then delete m
                    end repeat
                end repeat
            end repeat
            delay 0.2
        end repeat
        set trashed to id of (messages of trash mailbox whose subject is \(q(tag)))
        set remaining to 0
        repeat with a in accounts
            repeat with b in mailboxes of a
                repeat with m in (messages of b whose subject is \(q(tag)) and deleted status is false)
                    if (id of m) is not in trashed then set remaining to remaining + 1
                end repeat
            end repeat
        end repeat
        set remainingOutgoing to count (outgoing messages whose subject is \(q(tag)))
        return (remaining as string) & "|" & (remainingOutgoing as string)
        """
        if scenario == "MAIL-CLEANUP" {
            do { return .object(try cleanupResult(try await script(cleanup))) }
            catch {
                var result: [String: JSON] = ["cleanup_passed": .bool(false), "cleanup_error": .string(MailCommands.failureMessage(error))]
                if let failure = error as? MailCommandFailure, case .script(let code) = failure { result["cleanup_error_number"] = .number(Double(code)) }
                return .object(result)
            }
        }
        var checks: [String: JSON] = [:]
        do {
            let own = try await script("""
            set m to make new outgoing message with properties {subject:\(q(tag)), visible:false}
            set own to extract address from sender of m
            close m saving no
            return own
            """)
            guard own.contains("@"), !own.contains("\n") else { throw SourceFailure.unavailable }
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
                for _ in 0..<120 {
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
            do { checks.merge(try cleanupResult(try await script(cleanup))) { _, value in value } }
            catch {
                checks["cleanup_passed"] = .bool(false)
                checks["cleanup_error"] = .string(MailCommands.failureMessage(error))
                if let failure = error as? MailCommandFailure, case .script(let code) = failure { checks["cleanup_error_number"] = .number(Double(code)) }
            }
        }
        return .object(checks)
    }
}
#endif
