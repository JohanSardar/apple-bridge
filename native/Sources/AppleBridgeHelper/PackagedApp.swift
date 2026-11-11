import AppKit
import AppleSession
import ServiceManagement
import SourceSupport
import SwiftUI

@MainActor
final class PackagedApp: NSObject, NSApplicationDelegate {
    let service = SMAppService.agent(plistName: "dev.applebridge.gateway.plist")
    var arguments: [String] = []
    var connectorHome: String?
    var connectorLabExecutable: String?
    var connectorTask: Task<Void, Never>?
    let approvals = ApprovalModel()
    let approvalPanel = FloatingPanel(size: NSSize(width: 340, height: 360))

    nonisolated static func connectorCommand(_ arguments: [String], labExecutable: String?) -> Data? {
        guard let path = try? TrustedChild.bridge(labExecutable: labExecutable) else { return nil }
        return TrustedChild.capture(path, identifier: TrustedChild.bridgeName, arguments: ["connector"] + arguments)
    }

    nonisolated static func certificateWarning(home: String?) -> String? {
        guard let home else { return nil }
        struct Remote: Decodable { let ct_warning: String? }
        let file = URL(fileURLWithPath: home).appendingPathComponent("remote/status.json")
        guard let data = try? Data(contentsOf: file), let warning = (try? JSONDecoder().decode(Remote.self, from: data))?.ct_warning, !warning.isEmpty else { return nil }
        return warning
    }

    func watchConnectors() {
        let options = connectorHome.map { ["--home", $0] } ?? []
        let executable = connectorLabExecutable
        let warningHome = connectorHome
        approvals.decide = { [weak self] request, allow in self?.decide(request, allow: allow, options: options, executable: executable) }
        approvalPanel.contentView = NSHostingView(rootView: ApprovalView(model: approvals))
        connectorTask = Task { @MainActor [weak self] in
            var failures = 0
            var idle = 0
            while !Task.isCancelled {
                let data = await Task.detached { Self.connectorCommand(["pending"] + options, labExecutable: executable) }.value
                failures = data == nil ? failures + 1 : 0
                if failures >= 3 { NSApplication.shared.terminate(nil); return }
                guard let self else { return }
                let requests = data.flatMap { try? JSONDecoder().decode([ConnectorRequest].self, from: $0) } ?? []
                idle = requests.isEmpty && self.approvals.request == nil ? idle + 1 : 0
                if idle >= 30 { NSApplication.shared.terminate(nil); return }
                if let shown = self.approvals.request, let fresh = requests.first(where: { $0.id == shown.id }), fresh != shown {
                    self.approvals.request = fresh
                } else if let shown = self.approvals.request, !requests.contains(where: { $0.id == shown.id }) {
                    self.approvals.request = nil
                    self.approvalPanel.orderOut(nil)
                }
                if self.approvals.request == nil, let next = requests.first {
                    self.approvals.warning = Self.certificateWarning(home: warningHome) ?? ""
                    self.approvals.request = next
                    self.present()
                }
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    func present() {
        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        let size = approvalPanel.frame.size
        approvalPanel.setFrameOrigin(NSPoint(x: screen.maxX - size.width - 24, y: screen.midY - size.height / 2))
        approvalPanel.alphaValue = 0
        approvalPanel.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.25
            approvalPanel.animator().alphaValue = 1
        }, completionHandler: nil)
    }

    func decide(_ request: ConnectorRequest, allow: Bool, options: [String], executable: String?) {
        Task { @MainActor in
            let result = await Task.detached { Self.connectorCommand([allow ? "allow" : "deny", "--request", request.id, "--code", request.code] + options, labExecutable: executable) }.value
            approvals.decided = request.client_name
            approvals.request = nil
            approvals.outcome = result == nil ? "That request expired. Start again from the app." : allow ? "\(request.client_name) is connected" : "Denied"
            try? await Task.sleep(for: .seconds(1.6))
            if approvals.request == nil { approvalPanel.orderOut(nil) }
        }
    }

    static func run(_ arguments: [String]) throws {
        let controller = PackagedApp()
        controller.arguments = arguments
        if arguments.first == "connector-approvals", arguments.count >= 2 {
            controller.connectorHome = arguments[1]
            #if LAB
            if arguments.count == 3 { controller.connectorLabExecutable = arguments[2] }
            #endif
        }
        switch arguments.first {
        case "uninstall":
            try controller.service.unregister()
        case "service-status":
            print(controller.service.status.rawValue)
        case "install":
            try controller.install()
        default:
            BridgeIntentHost.runApplication(controller)
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
        guard TrustedChild.capture(try TrustedChild.bridge(), identifier: TrustedChild.bridgeName, arguments: credentialArguments) != nil else { throw NSError(domain: "AppleBridgeCredentials", code: 1) }
        try service.register()
        guard service.status == .enabled else {
            throw NSError(domain: "AppleBridgeBackgroundApprovalRequired", code: service.status.rawValue, userInfo: [NSLocalizedDescriptionKey: "Allow Apple Bridge in System Settings > General > Login Items & Extensions."])
        }
    }

    func show() {
        if connectorHome != nil { watchConnectors(); return }
        GuideController.start(arguments)
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        GuideController.retained?.panel.orderFrontRegardless()
        return true
    }
}
