import AppKit
import AppleSession
import ServiceManagement

@MainActor
final class PackagedApp: NSObject {
    let permissions = PermissionSession()
    let service = SMAppService.agent(plistName: "dev.applebridge.gateway.plist")
    var window: NSWindow?
    var details: NSTextField?

    static func run(_ arguments: [String]) throws {
        let controller = PackagedApp()
        switch arguments.first {
        case "uninstall":
            try controller.service.unregister()
        case "service-status":
            print(controller.service.status.rawValue)
        case "install":
            try controller.install()
        default:
            controller.show()
            NSApplication.shared.run()
            withExtendedLifetime(controller) {}
        }
    }

    func install() throws {
        let bundle = Bundle.main.bundleURL
        let plist = bundle.appendingPathComponent("Contents/Library/LaunchAgents/dev.applebridge.gateway.plist")
        let values = try PropertyListSerialization.propertyList(from: Data(contentsOf: plist), format: nil) as? [String: Any]
        let arguments = values?["ProgramArguments"] as? [String] ?? []
        var credentialArguments = ["install-credentials"]
        if let index = arguments.firstIndex(of: "--home"), index + 1 < arguments.count {
            credentialArguments += ["--home", arguments[index + 1]]
        }
        let process = Process()
        process.executableURL = bundle.appendingPathComponent("Contents/MacOS/bridge")
        process.arguments = credentialArguments
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw NSError(domain: "AppleBridgeCredentials", code: Int(process.terminationStatus)) }
        try service.register()
        guard service.status == .enabled else {
            throw NSError(domain: "AppleBridgeBackgroundApprovalRequired", code: service.status.rawValue, userInfo: [NSLocalizedDescriptionKey: "Allow Apple Bridge in System Settings > General > Login Items & Extensions."])
        }
    }

    func show() {
        NSApplication.shared.setActivationPolicy(.regular)
        let menu = NSMenu()
        let item = NSMenuItem()
        let application = NSMenu()
        application.addItem(withTitle: "Quit Apple Bridge", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        item.submenu = application
        menu.addItem(item)
        NSApplication.shared.mainMenu = menu
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 580, height: 410), styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "Apple Bridge"
        let details = NSTextField(wrappingLabelWithString: "")
        details.frame = NSRect(x: 24, y: 100, width: 532, height: 285)
        window.contentView?.addSubview(details)
        let button = NSButton(title: "Enable Background Sync and Permissions", target: self, action: #selector(enable))
        button.frame = NSRect(x: 24, y: 45, width: 390, height: 35)
        window.contentView?.addSubview(button)
        let refresh = NSButton(title: "Refresh", target: self, action: #selector(update))
        refresh.frame = NSRect(x: 435, y: 45, width: 110, height: 35)
        window.contentView?.addSubview(refresh)
        self.window = window
        self.details = details
        update()
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate()
    }

    @objc func update() {
        permissions.refresh()
        details?.stringValue = "Apple Bridge runs in the background after you quit.\n\n" + permissions.report.snapshot().sorted(by: { $0.key < $1.key }).map { "\($0.key.replacingOccurrences(of: "_", with: " ")): \($0.value)" }.joined(separator: "\n") + "\n\nBackground service: \(service.status == .enabled ? "enabled" : "not enabled")"
    }

    @objc func enable() {
        do { try install() } catch {
            details?.stringValue = String(describing: error)
            if service.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
            return
        }
        Task { @MainActor in
            await permissions.onboard()
            for (name, pane) in [("full_disk_access", "Privacy_AllFiles"), ("accessibility", "Privacy_Accessibility"), ("contacts", "Privacy_Contacts"), ("calendar", "Privacy_Calendars"), ("reminders", "Privacy_Reminders"), ("automation_messages", "Privacy_Automation"), ("automation_system_events", "Privacy_Automation")] {
                if permissions.report.snapshot()[name] != "granted", let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)") { NSWorkspace.shared.open(url) }
            }
            NSApplication.shared.setActivationPolicy(.regular)
            update()
        }
    }
}
