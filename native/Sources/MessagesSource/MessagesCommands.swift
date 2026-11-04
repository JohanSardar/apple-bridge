import Foundation
import SourceSupport

public actor MessagesCommands {
    private let path: String
    private let labCommands: Bool
    private let agentPolicy: URL?
    public init(path: String?, labCommands: Bool, agentPolicy: URL? = nil) {
        self.agentPolicy = agentPolicy
        self.path = path ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        self.labCommands = labCommands
    }
    private var osBuild: String { ProcessInfo.processInfo.operatingSystemVersionString }
    public var advertise: JSON { .object(["report_dir": .string(PrivateReport.directory.path), "os_build": .string(osBuild)]) }
    public func primitive(_ operation: String, args request: JSON) async throws -> JSON {
        switch operation {
        case "messages.config":
            var fields: [String: JSON] = ["lab_commands": .bool(labCommands), "chat_db": .string(path), "home": .string(FileManager.default.homeDirectoryForCurrentUser.path), "report_dir": .string(PrivateReport.directory.path), "os_build": .string(osBuild)]
            if let agentPolicy { fields["agent_policy"] = .string(agentPolicy.path) }
            return .object(fields)
        case "messages.me_card":
            return .object(["handles": .array(MessagesProbes.meCard().map(JSON.string))])
        case "messages.send_part":
            let error = await MessagesUI.send(request["part"], route: request["route"])
            return .object(["error": .number(Double(error))])
        case "messages.delete_owned_item":
            try await MessagesNativeSession.deleteOwnedItem(request["guid"].string, chat: request["chat"].string)
            return .object([:])
        default:
            if operation.hasPrefix("messages.ax_") { return try await MessagesAXPrimitives.answer(operation, args: request) }
            throw SourceFailure.unsupported
        }
    }
}

@UIActor
public enum MessagesUI {
    public static func execute(_ script: String) -> Int { MessagesAppleScript.execute(script) }
    public static func send(_ part: JSON, route: JSON) -> Int {
        let content = part["kind"].string == "text" ? MessagesAppleScript.literal(part["text"].string) : "POSIX file " + MessagesAppleScript.literal(part["path"].string)
        if route["kind"].string == "chat_id_raw" {
            return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\" to send \(content) to chat id \(MessagesAppleScript.literal(route["chat"].string))")
        }
        return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\"\nset targetService to first service whose service type = iMessage\nsend \(content) to buddy \(MessagesAppleScript.literal(route["handle"].string)) of targetService\nend tell")
    }
}
