import AppIntents

struct HealthLogIntent: AppIntent {
    static let title: LocalizedStringResource = "Log Health Sample"
    static let description = IntentDescription("Record the requested Health sample.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.health.log", arguments: arguments))
    }
}
