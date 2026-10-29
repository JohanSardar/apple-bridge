import AppIntents
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
        guard (try? JSONSerialization.jsonObject(with: data)) is [String: Any] else {
            throw CommandIntentError.invalidArguments
        }
        guard let handler = CommandHandlers.all[command] else { throw CommandIntentError.unavailable }
        return String(decoding: try await handler(data), as: UTF8.self)
    }
}

struct BridgeShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: CreateAlarmIntent(), phrases: ["Create Alarm with \(.applicationName)"], shortTitle: "Create Alarm", systemImageName: "iphone")
        AppShortcut(intent: ListAlarmsIntent(), phrases: ["List Alarms with \(.applicationName)"], shortTitle: "List Alarms", systemImageName: "iphone")
        AppShortcut(intent: UpdateAlarmIntent(), phrases: ["Update Alarm with \(.applicationName)"], shortTitle: "Update Alarm", systemImageName: "iphone")
        AppShortcut(intent: CancelAlarmIntent(), phrases: ["Cancel Alarm with \(.applicationName)"], shortTitle: "Cancel Alarm", systemImageName: "iphone")
        AppShortcut(intent: StartTimerIntent(), phrases: ["Start Timer with \(.applicationName)"], shortTitle: "Start Timer", systemImageName: "iphone")
        AppShortcut(intent: CancelTimerIntent(), phrases: ["Cancel Timer with \(.applicationName)"], shortTitle: "Cancel Timer", systemImageName: "iphone")
        AppShortcut(intent: MotionActivityIntent(), phrases: ["Get Motion Activity with \(.applicationName)"], shortTitle: "Get Motion Activity", systemImageName: "iphone")
        AppShortcut(intent: MotionHistoryIntent(), phrases: ["Get Motion History with \(.applicationName)"], shortTitle: "Get Motion History", systemImageName: "iphone")
        AppShortcut(intent: PingIntent(), phrases: ["Ping iPhone with \(.applicationName)"], shortTitle: "Ping iPhone", systemImageName: "iphone")
    }
}
