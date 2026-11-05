import Foundation
import SourceSupport

public actor MailSource: Source {
    public nonisolated let name = "mail"
    private let commands: MailCommands
    public static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("account settings, recipients, attachment files and local body availability require qualification")])])
    public init(script: @escaping MailCommands.Script = MailCommands.run) {
        commands = MailCommands(script: script)
    }
    public func open() async -> SourceReadiness { SourceReadiness(state: "ready", coverage: Self.coverage, capabilities: MailCommands.operations) }
    public func prepare(_ command: JSON) async throws -> Prepared { return try commands.prepare(command) }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        do { return try await commands.dispatch(prepared) }
        catch let error as MailCommandFailure {
            return .object(["observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string(error.localizedDescription)])
        }
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
