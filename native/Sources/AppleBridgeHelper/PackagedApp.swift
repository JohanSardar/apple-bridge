import AppKit
import AppleSession
import ServiceManagement

@MainActor
final class PackagedApp: NSObject {
    let permissions = PermissionSession()
    let service = SMAppService.agent(plistName: "dev.applebridge.gateway.plist")
    var window: NSWindow?
    var details: NSTextField?
    var connectorHome: String?
    var connectorTask: Task<Void, Never>?

    nonisolated static func connectorCommand(_ arguments: [String]) -> Data? {
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/bridge")
        process.arguments = ["connector"] + arguments
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            return process.terminationStatus == 0 ? data : nil
        } catch { return nil }
    }

    struct ConnectorRequest: Decodable, Sendable {
        let id: String
        let code: String
        let client_name: String
        let redirect_host: String
        let scopes: [String]
        let expires_at: String
    }

    func watchConnectors() {
        var homeArguments: [String] = []
        if let home = connectorHome { homeArguments = ["--home", home] }
        else {
            let plist = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/LaunchAgents/dev.applebridge.gateway.plist")
            if let data = try? Data(contentsOf: plist), let values = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any], let arguments = values["ProgramArguments"] as? [String], let index = arguments.firstIndex(of: "--home"), index + 1 < arguments.count { homeArguments = ["--home", arguments[index + 1]] }
        }
        let options = homeArguments
        connectorTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                let data = await Task.detached { Self.connectorCommand(["pending"] + options) }.value
                if let self, let data, let requests = try? JSONDecoder().decode([ConnectorRequest].self, from: data), let request = requests.first, let window = self.window, window.attachedSheet == nil {
                    let alert = NSAlert()
                    alert.messageText = "Allow \(request.client_name) to connect?"
                    alert.informativeText = "Code \(request.code)\n\nMatch this code in your browser.\nRequested data: \(request.scopes.joined(separator: ", "))\nReturns to: \(request.redirect_host)\nExpires in five minutes."
                    alert.addButton(withTitle: "Allow")
                    alert.addButton(withTitle: "Deny")
                    alert.buttons.first?.keyEquivalent = ""
                    window.makeKeyAndOrderFront(nil)
                    NSApplication.shared.activate()
                    alert.beginSheetModal(for: window) { response in
                        let action = response == .alertFirstButtonReturn ? "allow" : "deny"
                        Task { @MainActor in
                            let result = await Task.detached { Self.connectorCommand([action, "--request", request.id, "--code", request.code] + options) }.value
                            if result == nil { self.details?.stringValue = "Approval was refused or expired. Start again from your connector." }
                        }
                    }
                }
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    static func run(_ arguments: [String]) throws {
        let controller = PackagedApp()
        if arguments.first == "connector-approvals", arguments.count == 2 { controller.connectorHome = arguments[1] }
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
        watchConnectors()
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
