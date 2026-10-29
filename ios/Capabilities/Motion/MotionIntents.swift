import AppIntents

struct MotionActivityIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Motion Activity"
    static let description = IntentDescription("Run iphone.motion.activity on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.motion.activity"))
    }
}

struct MotionHistoryIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Motion History"
    static let description = IntentDescription("Run iphone.motion.history on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.motion.history", arguments: arguments))
    }
}

