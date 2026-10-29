import Foundation

enum CommandIntentError: Error, LocalizedError {
    case unavailable
    case invalidArguments

    var errorDescription: String? {
        switch self {
        case .unavailable: "Enable this capability in Apple Bridge before running its shortcut."
        case .invalidArguments: "Arguments must be a JSON object."
        }
    }
}

enum CommandIntent {
    @MainActor
    static func run(_ command: String, arguments: String = "{}") async throws -> String {
        let data = Data(arguments.utf8)
        guard var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw CommandIntentError.invalidArguments
        }
        guard let handler = CommandHandlers.all[command] else { throw CommandIntentError.unavailable }
        object["device_id"] = "iphone:local"
        let localData = try JSONSerialization.data(withJSONObject: object)
        return String(decoding: try await handler(localData), as: UTF8.self)
    }
}
