import AppIntents

struct PingIntent: AppIntent {
    static let title: LocalizedStringResource = "Check Bridge Status"
    static let description = IntentDescription("Check the bridge connection and device status.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        let value = try await CommandIntent.run("iphone.ping")
        #if DEBUG
        await ShellDeliveryQualification.shared.recordIntent(value)
        #endif
        return .result(value: value)
    }
}

