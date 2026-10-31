import AppIntents

struct CreateAlarmIntent: AppIntent {
    static let title: LocalizedStringResource = "Create Alarm"
    static let description = IntentDescription("Create an alarm with the requested time and label.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.alarm.create", arguments: arguments))
    }
}

struct ListAlarmsIntent: AppIntent {
    static let title: LocalizedStringResource = "List Alarms"
    static let description = IntentDescription("List the current alarms.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.alarm.list"))
    }
}

struct UpdateAlarmIntent: AppIntent {
    static let title: LocalizedStringResource = "Update Alarm"
    static let description = IntentDescription("Update the selected alarm.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.alarm.update", arguments: arguments))
    }
}

struct CancelAlarmIntent: AppIntent {
    static let title: LocalizedStringResource = "Cancel Alarm"
    static let description = IntentDescription("Cancel the selected alarm.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.alarm.cancel", arguments: arguments))
    }
}

struct StartTimerIntent: AppIntent {
    static let title: LocalizedStringResource = "Start Timer"
    static let description = IntentDescription("Start a timer with the requested duration.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.timer.start", arguments: arguments))
    }
}

struct CancelTimerIntent: AppIntent {
    static let title: LocalizedStringResource = "Cancel Timer"
    static let description = IntentDescription("Cancel the selected timer.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.timer.cancel", arguments: arguments))
    }
}

