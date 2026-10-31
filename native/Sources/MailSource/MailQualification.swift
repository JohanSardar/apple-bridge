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
    public func run(scenario: String, tag: String) async throws -> JSON {
        guard tag.range(of: #"^\[ab-test\] [A-Za-z0-9_-]+$"#, options: .regularExpression) != nil,
              ["MAIL-SEND-SELF-REAL", "MAIL-DRAFT-REAL", "MAIL-ORGANIZE-REAL", "MAIL-BODY-FIXTURE", "MAIL-CLEANUP"].contains(scenario) else { throw SourceFailure.invalid }
        let q = MailCommands.quote
        let cleanup = """
        repeat 2 times
            repeat with a in accounts
                repeat with b in mailboxes of a
                    set owned to (every message of b whose subject is \(q(tag)))
                    if (count owned) is greater than 0 then delete owned
                end repeat
            end repeat
        end repeat
        repeat with i from (count outgoing messages) to 1 by -1
            set m to outgoing message i
            if subject of m is \(q(tag)) then close m saving no
        end repeat
        set remaining to 0
        repeat with a in accounts
            repeat with b in mailboxes of a
                set remaining to remaining + (count (messages of b whose subject is \(q(tag)) and deleted status is false))
            end repeat
        end repeat
        set remaining to remaining + (count (outgoing messages whose subject is \(q(tag))))
        return (remaining is 0) as string
        """
        if scenario == "MAIL-CLEANUP" {
            do { return .object(["cleanup_passed": .bool(try await script(cleanup) == "true")]) }
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
            do { checks["cleanup_passed"] = .bool(try await script(cleanup) == "true") }
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
