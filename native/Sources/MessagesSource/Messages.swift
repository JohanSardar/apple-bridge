import Foundation
import SourceSupport

public actor Messages: Source {
    public nonisolated let name = "messages"
    private let commands: MessagesCommands
    private let attachmentHome: URL
    public nonisolated var attachmentsRoot: URL { attachmentHome.appendingPathComponent("Library/Messages/Attachments", isDirectory: true) }
    public init(labCommands: Bool = false, attachmentHome: URL = FileManager.default.homeDirectoryForCurrentUser) {
        self.attachmentHome = attachmentHome
        commands = MessagesCommands(path: nil, labCommands: labCommands)
    }
    public func open() async -> SourceReadiness { SourceReadiness(state: "ready", coverage: .object([:])) }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        try await commands.primitive(operation, args: args)
    }
}
