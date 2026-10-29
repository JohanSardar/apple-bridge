import Foundation

@MainActor
enum CommandHandlers {
    static var all: [String: IPhoneCommandHandler] {
        var handlers: [String: IPhoneCommandHandler] = [
            "iphone.ping": { _ in Data("{\"pong\":true}".utf8) },
        ]
        for capability in Capabilities.all where capability.available && UserDefaults.standard.bool(forKey: "capability.\(capability.id).enabled") {
            handlers.merge(capability.handlers, uniquingKeysWith: { _, new in new })
        }
        return handlers
    }
}
