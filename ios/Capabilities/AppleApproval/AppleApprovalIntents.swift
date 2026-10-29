import AppIntents

struct WalletAccountsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Wallet Accounts"
    static let description = IntentDescription("Run iphone.wallet.account on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.wallet.account", arguments: arguments))
    }
}

struct WalletTransactionsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Wallet Transactions"
    static let description = IntentDescription("Run iphone.wallet.transaction on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.wallet.transaction", arguments: arguments))
    }
}

struct CriticalAlertIntent: AppIntent {
    static let title: LocalizedStringResource = "Send Critical Alert"
    static let description = IntentDescription("Run iphone.critical.alert on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.critical.alert", arguments: arguments))
    }
}

struct LocationPushIntent: AppIntent {
    static let title: LocalizedStringResource = "Register Location Push"
    static let description = IntentDescription("Run iphone.location.push on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.push"))
    }
}

struct FallDetectionIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Fall Detection"
    static let description = IntentDescription("Run iphone.fall.detection on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.fall.detection"))
    }
}

