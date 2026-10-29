@preconcurrency import AlarmKit
import SwiftUI

@MainActor
final class SystemAlarmAdapter: AlarmAdapter {
    private let manager = AlarmManager.shared

    func authorize() async throws {
        let state = manager.authorizationState == .notDetermined
            ? try await manager.requestAuthorization() : manager.authorizationState
        guard state == .authorized else { throw AlarmCommandError.permission }
    }

    func list() throws -> [BridgeAlarm] {
        try manager.alarms.map { alarm in
            var fire: Double?
            if case .fixed(let date) = alarm.schedule { fire = date.timeIntervalSince1970 }
            return BridgeAlarm(id: alarm.id, fire_at: fire,
                               duration_seconds: alarm.countdownDuration?.preAlert,
                               snooze_seconds: alarm.countdownDuration?.postAlert ?? 540,
                               title: UserDefaults.standard.string(forKey: "alarm.title.\(alarm.id)") ?? "Apple Bridge",
                               state: String(describing: alarm.state))
        }
    }

    func schedule(_ alarm: BridgeAlarm) async throws -> BridgeAlarm {
        let title = LocalizedStringResource(stringLiteral: alarm.title)
        let presentation = AlarmPresentation(
            alert: .init(title: title,
                         stopButton: .init(text: "Stop", textColor: .white, systemImageName: "stop.circle"),
                         secondaryButton: .init(text: "Snooze", textColor: .white, systemImageName: "zzz"),
                         secondaryButtonBehavior: .countdown),
            countdown: .init(title: title))
        let attributes = AlarmAttributes(presentation: presentation, metadata: BridgeAlarmMetadata(), tintColor: .orange)
        let configuration = AlarmManager.AlarmConfiguration(
            countdownDuration: Alarm.CountdownDuration(preAlert: alarm.duration_seconds, postAlert: alarm.snooze_seconds),
            schedule: alarm.fire_at.map { .fixed(Date(timeIntervalSince1970: $0)) }, attributes: attributes)
        let scheduled = try await manager.schedule(id: alarm.id, configuration: configuration)
        UserDefaults.standard.set(alarm.title, forKey: "alarm.title.\(alarm.id)")
        var result = alarm
        result.state = String(describing: scheduled.state)
        return result
    }

    func cancel(_ id: UUID) throws {
        try manager.cancel(id: id)
        UserDefaults.standard.removeObject(forKey: "alarm.title.\(id)")
    }
}

enum AlarmsCapability {
    static let descriptor = Capability(id: "alarms", title: "Alarms and timers", permission: "AlarmKit Allow required", available: true,
        handlers: Dictionary(uniqueKeysWithValues: ["iphone.alarm.create", "iphone.alarm.list", "iphone.alarm.update", "iphone.alarm.cancel", "iphone.timer.start", "iphone.timer.cancel"].map { operation in
            (operation, { data in try await AlarmCommands(adapter: SystemAlarmAdapter()).execute(operation, data) } as IPhoneCommandHandler)
        }))
}
