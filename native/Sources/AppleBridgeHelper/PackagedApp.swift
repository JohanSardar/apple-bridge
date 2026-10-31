import AppKit
import AppleSession
import ServiceManagement
import SwiftUI

@MainActor
final class PackagedApp: NSObject, NSApplicationDelegate {
    let permissions = PermissionSession()
    let service = SMAppService.agent(plistName: "dev.applebridge.gateway.plist")
    var window: NSWindow?
    var setup: SetupModel?
    var setupHome: URL?
    var snapshotDirectory: URL?
    var connectorHome: String?
    var connectorExecutable: String?
    var connectorTask: Task<Void, Never>?

    nonisolated static func connectorCommand(_ arguments: [String], executable: String?) -> Data? {
        let process = Process()
        process.executableURL = executable.map { URL(fileURLWithPath: $0) } ?? Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/bridge")
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
        let executable = connectorExecutable
        connectorTask = Task { @MainActor [weak self] in
            var failures = 0
            while !Task.isCancelled {
                let data = await Task.detached { Self.connectorCommand(["pending"] + options, executable: executable) }.value
                failures = data == nil ? failures + 1 : 0
                if self?.connectorHome != nil, failures >= 3 { NSApplication.shared.terminate(nil); return }
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
                            let result = await Task.detached { Self.connectorCommand([action, "--request", request.id, "--code", request.code] + options, executable: executable) }.value
                            if result == nil { self.setup?.message = "Approval was refused or expired. Start again from your connector." }
                            await self.setup?.refresh()
                        }
                    }
                }
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    static func run(_ arguments: [String]) throws {
        let controller = PackagedApp()
        if arguments.first == "connector-approvals", arguments.count >= 2 {
            controller.connectorHome = arguments[1]
            if arguments.count == 3 { controller.connectorExecutable = arguments[2] }
        }
        if arguments.first == "setup-snapshot", arguments.count == 3 {
            controller.setupHome = URL(fileURLWithPath: arguments[1])
            controller.snapshotDirectory = URL(fileURLWithPath: arguments[2])
        }
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
        let setupItem = NSMenuItem()
        let setupMenu = NSMenu(title: "Setup")
        setupMenu.addItem(withTitle: "Open Setup", action: #selector(reopenSetup), keyEquivalent: "s").target = self
        setupItem.submenu = setupMenu
        menu.addItem(setupItem)
        NSApplication.shared.mainMenu = menu
        NSApplication.shared.delegate = self
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 860, height: 740), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Apple Bridge Setup"
        window.isReleasedWhenClosed = false
        let home = setupHome ?? gatewayHome()
        let model = SetupModel(home: home, testing: snapshotDirectory != nil)
        model.install = { [weak self] in try self?.install() }
        window.contentView = NSHostingView(rootView: SetupView(model: model))
        window.minSize = NSSize(width: 860, height: 740)
        self.window = window
        self.setup = model
        if snapshotDirectory == nil { model.start(); watchConnectors() }
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate()
        if let directory = snapshotDirectory {
            Task { @MainActor in
                do { try await self.captureSetup(model, directory: directory) }
                catch { FileHandle.standardError.write(Data("Setup snapshot failed.\n".utf8)); exit(1) }
                NSApplication.shared.terminate(nil)
            }
        }
    }

    func gatewayHome() -> URL {
        if let connectorHome { return URL(fileURLWithPath: connectorHome) }
        let plist = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/LaunchAgents/dev.applebridge.gateway.plist")
        if let data = try? Data(contentsOf: plist), let values = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any], let arguments = values["ProgramArguments"] as? [String], let index = arguments.firstIndex(of: "--home"), index + 1 < arguments.count { return URL(fileURLWithPath: arguments[index + 1]) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridge")
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        reopenSetup()
        return true
    }

    @objc func reopenSetup() {
        setup?.state.step = 0
        window?.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate()
    }

    func captureSetup(_ model: SetupModel, directory: URL) async throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        await model.refresh()
        let before = model.statuses
        for step in 0..<5 {
            model.state.step = step
            window?.contentView?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(400))
            guard let view = window?.contentView, let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { throw CocoaError(.fileWriteUnknown) }
            view.cacheDisplay(in: view.bounds, to: bitmap)
            guard let data = bitmap.representation(using: .png, properties: [:]) else { throw CocoaError(.fileWriteUnknown) }
            try data.write(to: directory.appendingPathComponent("step-\(step + 1).png"), options: .atomic)
        }
        await model.refresh()
        let tcc = model.permissions.compareTCC(model.statuses)
        let report: [String: Any] = ["tcc_matches": tcc.matches, "tcc_compared": tcc.count, "status_matches": before == model.statuses, "permission_count": model.statuses.count, "snapshot_count": 5, "request_calls": NewPermissions.requestAPICalls, "fresh_setup": !FileManager.default.fileExists(atPath: model.home.appendingPathComponent("setup.json").path)]
        try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]).write(to: directory.appendingPathComponent("report.json"), options: .atomic)
    }
}
