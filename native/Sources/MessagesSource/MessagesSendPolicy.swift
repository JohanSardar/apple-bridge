import Foundation
import SourceSupport

public enum MessagesSendPolicy {
    public static func tagged(_ text: String) -> Bool {
        text.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil
    }

    public static func ownsRecipients(own: Set<String>?, recipients: () throws -> [String]) throws -> Bool {
        guard let own else { return false }
        let all = try recipients()
        return !all.isEmpty && all.allSatisfy { own.contains(MessagesProbes.normalize($0)) }
    }

    public static func check(origin: CommandOrigin, text: String, fileCount: Int, own: () -> Set<String>?, recipients: () throws -> [String]) throws -> Bool {
        let ownerSelf = origin == .ownerSelf && !text.isEmpty && !tagged(text)
        if origin != .interactive && !text.isEmpty && !tagged(text) {
            guard ownerSelf, try ownsRecipients(own: own(), recipients: recipients) else { throw MessagesSendSafetyFailure.textTagRequired }
        }
        guard !text.isEmpty || fileCount > 0 else { throw SourceFailure.invalid }
        return ownerSelf
    }
}
