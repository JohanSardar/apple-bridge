import AppIntents
import ServiceManagement

struct BridgeStatusIntent: AppIntent {
    static let title: LocalizedStringResource = "Apple Bridge status"
    static let description = IntentDescription("Read whether Apple Bridge background sync is enabled.")
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let status = SMAppService.agent(plistName: "dev.applebridge.gateway.plist").status
        let message: String
        switch status {
        case .enabled:
            message = "Apple Bridge background service is enabled."
        case .requiresApproval:
            message = "Apple Bridge background service needs approval in System Settings."
        case .notRegistered:
            message = "Apple Bridge background service is not registered."
        case .notFound:
            message = "Apple Bridge background service configuration was not found."
        @unknown default:
            message = "Apple Bridge background service status is unknown."
        }
        return .result(value: message, dialog: IntentDialog(stringLiteral: message))
    }
}

struct BridgeShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: BridgeStatusIntent(), phrases: ["Check \(.applicationName) status"], shortTitle: "Bridge status", systemImageName: "checkmark.circle")
    }
}
