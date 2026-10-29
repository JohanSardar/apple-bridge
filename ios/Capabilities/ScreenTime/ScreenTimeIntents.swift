import AppIntents

struct ShieldAppsIntent: AppIntent {
    static let title: LocalizedStringResource = "Shield Selected Apps"
    static let description = IntentDescription("Run iphone.screentime.shield on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.screentime.shield"))
    }
}

struct UnshieldAppsIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove App Shields"
    static let description = IntentDescription("Run iphone.screentime.unshield on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.screentime.unshield"))
    }
}

struct ScheduleDowntimeIntent: AppIntent {
    static let title: LocalizedStringResource = "Schedule Downtime"
    static let description = IntentDescription("Run iphone.screentime.schedule on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.screentime.schedule", arguments: arguments))
    }
}

struct CancelDowntimeIntent: AppIntent {
    static let title: LocalizedStringResource = "Cancel Downtime"
    static let description = IntentDescription("Run iphone.screentime.cancel on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.screentime.cancel"))
    }
}

