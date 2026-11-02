import AppIntents

struct DeviceStatusIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Device Status"
    static let description = IntentDescription("Read the current device status.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.device.status"))
    }
}
