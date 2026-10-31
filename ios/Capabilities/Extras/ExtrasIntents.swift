import AppIntents

struct WeatherIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Weather"
    static let description = IntentDescription("Read the current weather.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.weather", arguments: arguments))
    }
}

struct JournalSuggestionsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Journal Suggestions"
    static let description = IntentDescription("Read available journal suggestions.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.journal.suggestions"))
    }
}

struct EnergyHintsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Energy Hints"
    static let description = IntentDescription("Read the current energy suggestions.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.energy.hints", arguments: arguments))
    }
}

