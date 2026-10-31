import AppIntents

struct WalletAccountsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Wallet Accounts"
    static let description = IntentDescription("Read authorized wallet account information.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.wallet.account", arguments: arguments))
    }
}

struct WalletTransactionsIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Wallet Transactions"
    static let description = IntentDescription("Read authorized wallet transactions.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.wallet.transaction", arguments: arguments))
    }
}

struct CriticalAlertIntent: AppIntent {
    static let title: LocalizedStringResource = "Send Critical Alert"
    static let description = IntentDescription("Send the requested critical alert.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.critical.alert", arguments: arguments))
    }
}

struct LocationPushIntent: AppIntent {
    static let title: LocalizedStringResource = "Register Location Push"
    static let description = IntentDescription("Request a location refresh.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.push"))
    }
}

struct FallDetectionIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Fall Detection"
    static let description = IntentDescription("Read the fall detection status.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.fall.detection"))
    }
}

