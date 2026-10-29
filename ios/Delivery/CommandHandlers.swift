import Foundation

@MainActor
enum CommandHandlers {
    static func handler(for op: String) -> IPhoneDeferredCommandHandler? {
        for capability in Capabilities.all where capability.available && UserDefaults.standard.bool(forKey: "capability.\(capability.id).enabled") {
            if let handler = capability.deferredHandlers[op] { return handler }
        }
        guard let handler = all[op] else { return nil }
        return { job in try await handler(JSONEncoder().encode(job.command.args)) }
    }

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
