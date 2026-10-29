import AppIntents

struct WeatherIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Weather"
    static let description = IntentDescription("Run iphone.weather on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.weather", arguments: arguments))
    }
}

struct JournalSuggestionsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Journal Suggestions"
    static let description = IntentDescription("Run iphone.journal.suggestions on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.journal.suggestions"))
    }
}

struct EnergyHintsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Energy Hints"
    static let description = IntentDescription("Run iphone.energy.hints on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.energy.hints", arguments: arguments))
    }
}

