import AppIntents

struct MotionActivityIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Motion Activity"
    static let description = IntentDescription("Read the current motion activity.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.motion.activity"))
    }
}

struct MotionHistoryIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Motion History"
    static let description = IntentDescription("Read motion history for the requested period.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.motion.history", arguments: arguments))
    }
}

