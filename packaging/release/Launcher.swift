import AppKit
import ServiceManagement
import Sparkle

@MainActor
final class ReleaseApplication: NSObject, NSApplicationDelegate, SPUUpdaterDelegate {
    var updater: SPUStandardUpdaterController?
    var window: NSWindow?
    var onboarding: Process?
    let agent = SMAppService.agent(plistName: "dev.applebridge.gateway.plist")

    func applicationDidFinishLaunching(_ notification: Notification) {
        updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)
        if UserDefaults.standard.bool(forKey: "R1RestartGateway") {
            do {
                if agent.status == .enabled {
                    try command("/bin/launchctl", ["kickstart", "-k", "gui/\(getuid())/dev.applebridge.gateway"])
                }
                UserDefaults.standard.removeObject(forKey: "R1RestartGateway")
            } catch { showError(error) }
        }
        let menu = NSMenu()
        let appItem = NSMenuItem()
        let actions = NSMenu()
        actions.addItem(withTitle: "Open Setup…", action: #selector(setup), keyEquivalent: "s").target = self
        actions.addItem(withTitle: "Check for Updates…", action: #selector(checkUpdates), keyEquivalent: "u").target = self
        actions.addItem(withTitle: "Quit Apple Bridge", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = actions
        menu.addItem(appItem)
        NSApplication.shared.mainMenu = menu
        let panel = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 300), styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        panel.title = "Apple Bridge"
        let text = NSTextField(wrappingLabelWithString: "Set up Apple Bridge, keep it up to date, and choose whether it starts when you log in.")
        text.frame = NSRect(x: 24, y: 225, width: 472, height: 50)
        panel.contentView?.addSubview(text)
        for (index, action) in [("Set Up Permissions and Sync…", #selector(setup)), ("Check for Updates…", #selector(checkUpdates)), ("Toggle Open at Login", #selector(toggleLogin)), ("Uninstall Apple Bridge…", #selector(uninstall))].enumerated() {
            let button = NSButton(title: action.0, target: self, action: action.1)
            button.frame = NSRect(x: 24, y: 175 - index * 45, width: 472, height: 34)
            panel.contentView?.addSubview(button)
        }
        window = panel
        panel.center()
        panel.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate()
        if !UserDefaults.standard.bool(forKey: "Q1SetupOpened") { setup(); UserDefaults.standard.set(true, forKey: "Q1SetupOpened") }
    }
    func updater(_ updater: SPUUpdater, willInstallUpdate item: SUAppcastItem) {
        UserDefaults.standard.set(agent.status == .enabled, forKey: "R1RestartGateway")
        onboarding?.terminate()
    }
    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        UserDefaults.standard.removeObject(forKey: "R1RestartGateway")
    }
    func applicationWillTerminate(_ notification: Notification) {
        onboarding?.terminate()
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        window?.makeKeyAndOrderFront(nil)
        return true
    }
    func showError(_ error: Error) {
        let alert = NSAlert()
        alert.messageText = "Apple Bridge needs your attention"
        alert.informativeText = error.localizedDescription
        alert.runModal()
    }
    @objc func setup() {
        if let onboarding, onboarding.isRunning { NSRunningApplication(processIdentifier: onboarding.processIdentifier)?.activate(); return }
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/AppleBridgeHelper")
        do { try process.run(); onboarding = process } catch { showError(error) }
    }
    @objc func checkUpdates() { updater?.checkForUpdates(nil) }
    @objc func toggleLogin() {
        do {
            if SMAppService.mainApp.status == .enabled {
                try SMAppService.mainApp.unregister()
            } else {
                try SMAppService.mainApp.register()
                if SMAppService.mainApp.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
            }
            let alert = NSAlert()
            alert.messageText = SMAppService.mainApp.status == .enabled ? "Open at login is on" : "Open at login is off"
            alert.runModal()
        } catch { showError(error) }
    }
    func command(_ path: String, _ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = arguments
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw ReleaseError.cleanupFailed }
    }
    @objc func uninstall() {
        let alert = NSAlert()
        alert.messageText = "Uninstall Apple Bridge?"
        alert.informativeText = "This stops sync, removes login items, deletes Apple Bridge data and saved permissions, and moves the app to the Trash. Remove any remaining Full Disk Access and Accessibility entries in System Settings > Privacy & Security. Your Apple app data stays in place."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Uninstall")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        do {
            let home = FileManager.default.homeDirectoryForCurrentUser
            try ReleaseCleanup.run(home: home, unregister: {
                if agent.status != .notRegistered { try agent.unregister() }
                if SMAppService.mainApp.status != .notRegistered { try SMAppService.mainApp.unregister() }
                onboarding?.terminate()
            }, removeCredentials: {
                let keychain = home.appendingPathComponent("Library/Application Support/AppleBridge/Apple Bridge.keychain-db")
                if FileManager.default.fileExists(atPath: keychain.path) { try command("/usr/bin/security", ["delete-keychain", keychain.path]) }
            }, resetPermissions: {
                try command("/usr/bin/tccutil", ["reset", "All", "dev.applebridge.helper"])
                updater?.updater.automaticallyChecksForUpdates = false
                updater?.updater.automaticallyDownloadsUpdates = false
                UserDefaults.standard.removePersistentDomain(forName: "dev.applebridge.helper")
                UserDefaults.standard.synchronize()
            })
            NSWorkspace.shared.recycle([Bundle.main.bundleURL]) { _, error in
                Task { @MainActor in
                    if let error { self.showError(error) } else { NSApplication.shared.terminate(nil) }
                }
            }
        } catch { showError(error) }
    }
}

@main
struct ReleaseLauncher {
    @MainActor static func main() {
        let app = NSApplication.shared
        let controller = ReleaseApplication()
        app.setActivationPolicy(.regular)
        app.delegate = controller
        app.run()
        withExtendedLifetime(controller) {}
    }
}
