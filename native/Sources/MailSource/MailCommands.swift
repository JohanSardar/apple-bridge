import Foundation
import CryptoKit
import Carbon
import AppKit
import SourceSupport

public enum MailCommandFailure: Error, LocalizedError, Sendable {
    case permission, timeout, rejected
    public static func appleEvent(_ code: Int) -> Self {
        switch code {
        case -1712: .timeout
        case -1743, -1744: .permission
        default: .rejected
        }
    }
    public var errorDescription: String? {
        switch self {
        case .permission: "Mail automation is not allowed. Enable Mail for Apple Bridge in System Settings > Privacy & Security > Automation."
        case .timeout: "Mail did not answer the command in time. Open Mail, let account sync finish, and try again."
        case .rejected: "Mail could not complete the command. Open Mail, check the account and mailbox, and try again."
        }
    }
}

public struct MailCommands: Sendable {
    public typealias Script = @Sendable (String) async throws -> String
    private let script: Script
    public static let operations = ["mail.create_draft", "mail.send", "mail.move", "mail.set_flag", "mail.mark_read", "mail.delete"]
    public init(script: @escaping Script = MailCommands.run) { self.script = script }
    @MainActor public static func run(_ body: String) async throws -> String {
        if NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail").isEmpty {
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.mail") else { throw MailCommandFailure.rejected }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = false
            do { _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration) }
            catch { throw MailCommandFailure.rejected }
        }
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.mail")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        guard permission == noErr else { throw MailCommandFailure.appleEvent(Int(permission)) }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 10 seconds\ntell application id \"com.apple.mail\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        if let error { throw MailCommandFailure.appleEvent(error["NSAppleScriptErrorNumber"] as? Int ?? 0) }
        return result.stringValue ?? ""
    }
    public static func failureMessage(_ error: any Error) -> String {
        if let failure = error as? MailCommandFailure { return failure.localizedDescription }
        if error is CancellationError { return "The Mail command was cancelled. Check Mail before trying again." }
        return MailCommandFailure.rejected.localizedDescription
    }
    public static func quote(_ value: String) -> String {
        "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\n", with: "\\n") + "\""
    }
    public func prepare(_ command: JSON) throws -> Prepared {
        let action = command["action"], args = action["args"], op = action["op"].string
        guard Self.operations.contains(op) else { throw SourceFailure.unsupported }
        if ["mail.create_draft", "mail.send"].contains(op) {
            guard !args["to"].array.isEmpty, !args["subject"].string.isEmpty else { throw SourceFailure.invalid }
            for address in args["to"].array + args["cc"].array { guard address.string.contains("@"), !address.string.contains("\n") else { throw SourceFailure.invalid } }
        } else {
            guard UInt64(action["native_key"].string) != nil else { throw SourceFailure.invalid }
        }
        guard args["attachments"].array.isEmpty else { throw SourceFailure.unsupported }
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        return Prepared(nativeState: .null, fingerprint: SHA256.hash(data: try encoder.encode(action)).map { String(format: "%02x", $0) }.joined(), action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let action = prepared.action, args = action["args"], op = action["op"].string
        let q = Self.quote
        var body = "", readback = ""
        var creates = false
        if op == "mail.create_draft" || op == "mail.send" {
            if !args["reply_to"].string.isEmpty || !args["forward_of"].string.isEmpty { throw SourceFailure.unsupported }
            creates = true
            body = "set m to make new outgoing message with properties {subject:" + q(args["subject"].string) + ", content:" + q(args["body"].string) + ", visible:false}\n"
            for (field, kind) in [("to", "to"), ("cc", "cc")] {
                for address in args[field].array { body += "tell m to make new " + kind + " recipient at end of " + kind + " recipients with properties {address:" + q(address.string) + "}\n" }
            }
            body += op == "mail.send" ? "send m\n" : "save m\n"
            readback = "return count of (messages of " + (op == "mail.send" ? "sent mailbox" : "drafts mailbox") + " whose subject is " + q(args["subject"].string) + " and content starts with " + q(args["body"].string) + ")"
        } else {
            let key = action["native_key"].string
            guard UInt64(key) != nil else { throw SourceFailure.invalid }
            let lookup = "set m to missing value\nrepeat with a in accounts\nrepeat with b in mailboxes of a\nset matches to (messages of b whose id is " + key + ")\nif (count of matches) is greater than 0 then set m to item 1 of matches\nend repeat\nend repeat\nif m is missing value then error \"message unavailable\"\n"
            body = lookup
            switch op {
            case "mail.mark_read":
                let value = args["read"].bool ? "true" : "false"
                body += "set read status of m to " + value + "\n"
                readback = lookup + "return (read status of m is " + value + ") as string"
            case "mail.set_flag":
                let value = args["flagged"].bool ? "true" : "false"
                body += "set flagged status of m to " + value + "\n"
                readback = lookup + "return (flagged status of m is " + value + ") as string"
            default:
                let target = op == "mail.delete" ? "trash mailbox" : "mailbox " + q(args["mailbox"].string) + " of account " + q(args["account"].string)
                body += "move m to " + target + "\n"
                readback = "return count of (messages of " + target + " whose id is " + key + ")"
            }
        }
        if creates {
            guard try await script(readback) == "0" else { throw SourceFailure.conflict }
        }
        _ = try await script(body)
        for _ in 0..<40 {
            let evidence = try await script(readback)
            if evidence == "1" || evidence == "true" { return .object(["observed": .bool(true)]) }
            try await Task.sleep(for: .milliseconds(200))
        }
        return .object(["observed": .bool(false), "disposition": .string("indeterminate")])
    }
}
