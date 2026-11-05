import Foundation
import SourceSupport

public struct MailSource: Source {
    public let name = "mail"
    public init() {}
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard operation == "mail.script" else { throw SourceFailure.unsupported }
        return try await AppleScriptRunner.run(args["source"].string, target: "com.apple.mail")
    }
}
