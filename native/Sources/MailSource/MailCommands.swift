import Foundation
import CryptoKit
import Carbon
import AppKit
import SourceSupport

public enum MailCommandFailure: Error, LocalizedError, Sendable, Equatable {
    case permission, timeout, connectionLost, rejected, unconfirmed
    case script(Int)
    public static func appleEvent(_ code: Int) -> Self {
        switch code {
        case -1712: .timeout
        case -1743, -1744: .permission
        case -600, -609: .connectionLost
        default: .script(code)
        }
    }
    public var errorDescription: String? {
        switch self {
        case .permission: "Mail automation is not allowed. Enable Mail for Apple Bridge in System Settings > Privacy & Security > Automation."
        case .timeout: "Mail automation did not respond in time. Restart Mail and try again. If it still does not respond, sign out of macOS and sign in again."
        case .connectionLost: "Mail closed before completing the command. Reopen Mail, let mailbox sync finish, and check the message before trying again."
        case .rejected, .script: "Mail could not complete the command. Open Mail, check the account and mailbox, and try again."
        case .unconfirmed: "Mail could not confirm the message after the command. Check Sent or Drafts before trying again."
        }
    }
}

private final class MailPermissionReply: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Void, any Error>?
    init(_ continuation: CheckedContinuation<Void, any Error>) { self.continuation = continuation }
    var pending: Bool { lock.withLock { continuation != nil } }
    func finish(_ failure: MailCommandFailure?) {
        let reply = lock.withLock {
            let reply = continuation
            continuation = nil
            return reply
        }
        if let failure { reply?.resume(throwing: failure) }
        else { reply?.resume() }
    }
}

public struct MailCommands: Sendable {
    private static let permissionQueue = DispatchQueue(label: "apple-bridge.mail-automation-permission")
    public typealias Script = @Sendable (String) async throws -> String
    private let script: Script
    public static let operations = ["mail.create_draft", "mail.send", "mail.move", "mail.set_flag", "mail.mark_read", "mail.delete"]
    public init(script: @escaping Script = MailCommands.run) { self.script = script }
    @MainActor public static func run(_ body: String) async throws -> String {
        try Task.checkCancellation()
        if NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.mail").isEmpty {
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.mail") else { throw MailCommandFailure.rejected }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = false
            do { _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration) }
            catch { throw MailCommandFailure.rejected }
        }
        try await requireAutomationPermission {
            let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.mail")
            return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 15 seconds\ntell application id \"com.apple.mail\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        if let error { throw MailCommandFailure.appleEvent(error["NSAppleScriptErrorNumber"] as? Int ?? 0) }
        return result.stringValue ?? ""
    }
    public static func requireAutomationPermission(after interval: TimeInterval = 10, check: @escaping @Sendable () -> Int32) async throws {
        try Task.checkCancellation()
        try await withCheckedThrowingContinuation { continuation in
            let reply = MailPermissionReply(continuation)
            permissionQueue.async {
                guard reply.pending else { return }
                let status = check()
                reply.finish(status == noErr ? nil : MailCommandFailure.appleEvent(Int(status)))
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + interval) {
                reply.finish(.timeout)
            }
        }
        try Task.checkCancellation()
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
        var body = "", readback = "", identity = ""
        var creates = false
        if op == "mail.create_draft" || op == "mail.send" {
            if !args["reply_to"].string.isEmpty || !args["forward_of"].string.isEmpty { throw SourceFailure.unsupported }
            creates = true
            body = "set m to make new outgoing message with properties {subject:" + q(args["subject"].string) + ", content:" + q(args["body"].string) + ", visible:false}\n"
            for (field, kind) in [("to", "to"), ("cc", "cc")] {
                for address in args[field].array { body += "tell m to make new " + kind + " recipient at end of " + kind + " recipients with properties {address:" + q(address.string) + "}\n" }
            }
            body += op == "mail.send" ? "send m\n" : "save m\nset visible of m to true\nclose m saving yes\n"
            let selection = "messages of " + (op == "mail.send" ? "sent mailbox" : "drafts mailbox") + " whose subject is " + q(args["subject"].string)
            let matches = """
            set matched to {}
            set expectedBody to \(q(args["body"].string))
            repeat with candidate in (\(selection))
                set candidateBody to (content of candidate) as string
                if candidateBody starts with expectedBody or candidateBody starts with (linefeed & expectedBody) or candidateBody starts with (return & expectedBody) or candidateBody starts with (return & linefeed & expectedBody) then set end of matched to contents of candidate
            end repeat

            """
            readback = matches + "return count of matched"
            identity = matches + "if (count matched) is not 1 then return \"\"\nreturn (id of item 1 of matched) as string"
        } else {
            let key = action["native_key"].string
            guard UInt64(key) != nil else { throw SourceFailure.invalid }
            let lookup = "set m to missing value\nrepeat with a in accounts\nrepeat with b in mailboxes of a\nset matches to (messages of b whose id is " + key + ")\nif (count of matches) is greater than 0 then set m to item 1 of matches\nif m is not missing value then exit repeat\nend repeat\nif m is not missing value then exit repeat\nend repeat\nif m is missing value then error \"message unavailable\"\n"
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
        let deadline = ContinuousClock.now.advanced(by: .seconds(15))
        for _ in 0..<40 {
            try Task.checkCancellation()
            guard ContinuousClock.now < deadline else { throw MailCommandFailure.timeout }
            let evidence = try await script(readback)
            if evidence == "1" || evidence == "true" {
                let key = creates ? try await script(identity) : action["native_key"].string
                guard UInt64(key) != nil else { throw MailCommandFailure.unconfirmed }
                return .object(["observed": .bool(true), "native_key": .string(key), "fingerprint": .string(prepared.fingerprint)])
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        return .object(["observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string(MailCommandFailure.unconfirmed.localizedDescription)])
    }
}
