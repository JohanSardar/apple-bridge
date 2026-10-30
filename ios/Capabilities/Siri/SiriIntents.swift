import AppIntents

struct PingIntent: AppIntent {
    static let title: LocalizedStringResource = "Ping iPhone"
    static let description = IntentDescription("Run iphone.ping on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        let value = try await CommandIntent.run("iphone.ping")
        #if DEBUG
        await ShellDeliveryQualification.shared.recordIntent(value)
        #endif
        return .result(value: value)
    }
}

