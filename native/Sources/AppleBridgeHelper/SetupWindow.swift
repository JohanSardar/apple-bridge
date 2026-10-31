import AppKit
import SwiftUI
import CoreImage.CIFilterBuiltins

@MainActor
final class SetupModel: ObservableObject {
    @Published var state = SetupState()
    @Published var statuses: [String: String] = [:]
    @Published var message = ""
    @Published var qr: NSImage?
    @Published var busy: Set<String> = []
    @Published var remoteReady = false
    @Published var syncEnabled = false
    @Published var connectorURL = ""
    let home: URL
    let permissions = SetupPermissions()
    var install: (() throws -> Void)?
    var syncStatus: (() -> Bool)?
    var refreshTask: Task<Void, Never>?
    var pairingExpiry: Date?
    var runCommand: @Sendable ([String], String) -> Data? = { arguments, home in SetupModel.command(arguments, home: home) }
    var openURL: (URL) -> Void = { NSWorkspace.shared.open($0) }
    var copyURL: (String) -> Void = { value in
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
    }
    let testing: Bool

    init(home: URL, testing: Bool = false) {
        self.home = home
        self.testing = testing
        if let data = try? Data(contentsOf: home.appendingPathComponent("setup.json")), let saved = try? JSONDecoder().decode(SetupState.self, from: data) {
            state = saved
            state.step = min(max(state.step, 0), 4)
        }
    }
    func save() {
        try? FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        if let data = try? JSONEncoder().encode(state) { try? data.write(to: home.appendingPathComponent("setup.json"), options: [.atomic, .completeFileProtectionUnlessOpen]) }
    }
    func move(_ forward: Bool) {
        if forward { state.advance() } else { state.back() }
        message = ""
        save()
    }
    func skip(_ id: String) {
        if state.skipped.contains(id) { state.skipped.remove(id) } else { state.skipped.insert(id) }
        save()
    }
    func start() {
        guard refreshTask == nil else { return }
        refreshTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.refresh()
                do { try await Task.sleep(for: .seconds(3)) } catch { return }
            }
        }
    }
    func stop() { refreshTask?.cancel(); refreshTask = nil }
    func refresh() async {
        statuses = await permissions.status()
        syncEnabled = syncStatus?() ?? false
        if let expiry = pairingExpiry, expiry < Date() { qr = nil; message = "Pairing code expired. Click Show Pairing QR to try again, or continue without a phone."; pairingExpiry = nil }
        guard !testing else { return }
        struct Remote: Decodable { let state: String; let url: String }
        if let data = try? Data(contentsOf: home.appendingPathComponent("remote/status.json")), let remote = try? JSONDecoder().decode(Remote.self, from: data), remote.state == "on", let url = URL(string: remote.url), url.scheme == "https", url.host != nil {
            remoteReady = true
            connectorURL = remote.url.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/mcp"
        } else { remoteReady = false; connectorURL = "" }
        struct Principal: Decodable {
            let kind: String
            let name: String?
            let iphone_signing_key: String?
            let revoked_at: String?
            let expires_at: String?
        }
        struct Credentials: Decodable { let principals: [Principal] }
        state.phone = false
        state.connectors = []
        if let data = try? Data(contentsOf: home.appendingPathComponent("credentials.json")), let credentials = try? JSONDecoder().decode(Credentials.self, from: data) {
            let active = credentials.principals.filter { principal in
                guard principal.revoked_at == nil else { return false }
                guard let expiry = principal.expires_at, !expiry.hasPrefix("0001-") else { return true }
                let fractional = ISO8601DateFormatter()
                fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
                return (fractional.date(from: expiry) ?? ISO8601DateFormatter().date(from: expiry)).map { $0 > Date() } ?? false
            }
            state.phone = active.contains { $0.iphone_signing_key != nil }
            state.connectors = Set(["Claude.ai", "ChatGPT"].filter { provider in
                active.contains { $0.kind == "agent" && ($0.name ?? "").lowercased().contains(provider == "Claude.ai" ? "claude" : "chatgpt") }
            })
        }
    }
    func request(_ permission: SetupPermission) {
        guard !testing, busy.insert(permission.id).inserted else { return }
        state.skipped.remove(permission.id)
        Task { @MainActor in
            defer { busy.remove(permission.id) }
            do { try await permissions.request(permission) }
            catch { message = "Permission was not granted. " + permission.fix }
            await refresh()
            save()
        }
    }
    nonisolated static func command(_ arguments: [String], home: String) -> Data? {
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/bridge")
        process.arguments = arguments + ["--home", home]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            let timer = DispatchSource.makeTimerSource(queue: .global())
            timer.schedule(deadline: .now() + 60)
            timer.setEventHandler {
                if process.isRunning { process.terminate() }
                DispatchQueue.global().asyncAfter(deadline: .now() + 5) {
                    if process.isRunning { kill(process.processIdentifier, SIGKILL) }
                }
            }
            timer.resume()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            timer.cancel()
            return process.terminationStatus == 0 ? data : nil
        } catch { return nil }
    }
    func enableSync() {
        guard !testing, busy.insert("sync").inserted else { return }
        let path = home.path
        let run = runCommand
        Task { @MainActor in
            defer { busy.remove("sync") }
            let data = await Task.detached { run(["install-credentials"], path) }.value
            do {
                guard data != nil else { throw CocoaError(.fileWriteUnknown) }
                try install?()
                syncEnabled = syncStatus?() ?? false
                message = syncEnabled ? "Background sync is on." : "Allow Apple Bridge in System Settings > General > Login Items & Extensions, then try again. You can continue for now."
            } catch { message = "Sync could not start. Move Apple Bridge to Applications, then allow it in System Settings > General > Login Items & Extensions. You can continue and try again later." }
        }
    }
    func enableRemote() {
        guard !testing, busy.insert("remote").inserted else { return }
        message = "Follow the browser link to enable Remote. You can continue while it connects."
        let path = home.path
        let run = runCommand
        Task { @MainActor in
            let data = await Task.detached { run(["remote", "on"], path) }.value
            busy.remove("remote")
            await refresh()
            message = data == nil ? "Remote did not connect. Check your internet connection and try Enable Remote again. Local features still work." : "Remote is ready."
        }
    }
    func pair() {
        guard !testing, busy.insert("pair").inserted else { return }
        let path = home.path
        let run = runCommand
        Task { @MainActor in
            defer { busy.remove("pair") }
            guard let data = await Task.detached(operation: { run(["pair", "--iphone", "--via-remote"], path) }).value,
                  let code = String(data: data, encoding: .utf8)?.components(separatedBy: .newlines).first, !code.isEmpty else {
                message = "Pairing is not ready. Enable Remote, then try Show Pairing QR again. You can continue without a phone."
                return
            }
            let filter = CIFilter.qrCodeGenerator()
            filter.message = Data(code.utf8)
            guard let image = filter.outputImage, let cg = CIContext().createCGImage(image.transformed(by: CGAffineTransform(scaleX: 5, y: 5)), from: image.extent.applying(CGAffineTransform(scaleX: 5, y: 5))) else { message = "The QR code could not be shown. Try again, or continue without a phone."; return }
            qr = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
            pairingExpiry = Date().addingTimeInterval(600)
            message = "In Apple Bridge on iPhone, choose Pair Mac and scan this code. It expires in 10 minutes."
        }
    }
    func connect(_ provider: String) {
        guard !testing, remoteReady, !connectorURL.isEmpty else { message = "Enable Remote first. Then try Connect again. Local features still work."; return }
        copyURL(connectorURL)
        let page = provider == "Claude.ai" ? "https://claude.ai/settings/connectors" : "https://chatgpt.com/#settings/Connectors"
        openURL(URL(string: page)!)
        message = "Connector URL copied. Add a custom connector in \(provider) and paste it. Match the four-digit code, then click Allow on this Mac. Return here after connecting."
    }
}

struct SetupView: View {
    @ObservedObject var model: SetupModel
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                ForEach(0..<5) { step in
                    Text("\(model.state.completed.contains(step) || step == 4 && model.state.step == 4 ? "✓" : String(step + 1)) \(SetupState.titles[step])")
                        .font(.caption).foregroundStyle(step == model.state.step ? Color.accentColor : .secondary)
                    if step < 4 { Spacer() }
                }
            }
            Divider()
            Text(SetupState.titles[model.state.step]).font(.largeTitle.bold())
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    content
                    if !model.message.isEmpty { Text(model.message).foregroundStyle(.secondary).textSelection(.enabled) }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.trailing, 8)
            }
            Divider()
            HStack {
                if model.state.step > 0 { Button("Back") { model.move(false) } }
                Spacer()
                Text("All steps are optional. Reopen from the Setup menu.").font(.caption).foregroundStyle(.secondary)
                if model.state.step < 4 {
                    Button(model.state.step == 0 ? "Get Started" : "Continue") { model.move(true) }.keyboardShortcut(.defaultAction)
                } else {
                    Button("Finish") { model.state.completed.insert(4); model.save(); NSApp.keyWindow?.close() }.keyboardShortcut(.defaultAction)
                }
            }
        }.padding(24).frame(minWidth: 820, minHeight: 690)
            .background(Color(nsColor: .windowBackgroundColor))
            .buttonStyle(SetupButtonStyle())
    }
    @ViewBuilder var content: some View {
        switch model.state.step {
        case 0:
            Text("Your Apple apps, ready for your AI.").font(.title2)
            Text("Choose what to share, pair your iPhone, and connect your AI. You can skip anything and return later.")
            Text("Keep Apple Bridge in Applications. Enable background sync to keep your data up to date after you close this window.")
            Text(model.syncEnabled ? "✓ Background sync is on" : "Background sync is not enabled yet.")
            Button(model.busy.contains("sync") ? "Starting…" : "Enable Background Sync") { model.enableSync() }.disabled(model.busy.contains("sync"))
        case 1:
            Text("Allow only what you want to use. Status refreshes automatically. Open Settings never requests access; Request Access may show a system prompt.")
            ForEach(SetupPermission.all) { permission in
                VStack(alignment: .leading, spacing: 5) {
                    HStack {
                        Text(permission.title).font(.headline)
                        Spacer()
                        Text(model.state.label(permission.id, status: model.statuses[permission.id]))
                            .foregroundStyle(model.state.label(permission.id, status: model.statuses[permission.id]) == "Allowed" ? .green : .secondary)
                        Button("Open Settings") { NSWorkspace.shared.open(permission.settingsURL) }
                        if !["full_disk_access", "accessibility"].contains(permission.id) {
                            Button(model.busy.contains(permission.id) ? "Waiting…" : "Request Access") { model.request(permission) }.disabled(model.busy.contains(permission.id))
                        }
                        Button(model.state.skipped.contains(permission.id) ? "Undo Skip" : "Skip") { model.skip(permission.id) }
                    }
                    Text(permission.why).font(.subheadline)
                    if model.state.skipped.contains(permission.id) || ["denied", "restricted", "unknown", "target_not_running", "limited"].contains(model.statuses[permission.id] ?? "unknown") {
                        Text(permission.fix + (permission.bundle != nil ? " Open the target app first if it is not listed." : "")).font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(.vertical, 7)
                Divider()
            }
        case 2:
            Text(model.state.phone ? "✓ iPhone paired" : "Add your iPhone for phone data and approvals away from your Mac.")
            remoteControls
            Button(model.busy.contains("pair") ? "Preparing…" : "Show Pairing QR") { model.pair() }.disabled(model.busy.contains("pair"))
            if let qr = model.qr { Image(nsImage: qr).interpolation(.none).resizable().scaledToFit().frame(width: 300, height: 300).padding(12).background(.white) }
            Text("No iPhone? Continue to use your Mac on its own.").foregroundStyle(.secondary)
        case 3:
            Text("Connect your AI using a private approval on this Mac.")
            remoteControls
            ForEach(["Claude.ai", "ChatGPT"], id: \.self) { provider in
                HStack {
                    Button("Connect \(provider)") { model.connect(provider) }
                    Text(model.state.connectors.contains(provider) ? "✓ Connected" : "Not connected yet").foregroundStyle(model.state.connectors.contains(provider) ? .green : .secondary)
                }
            }
            Text("Each button copies your connector URL and opens connector settings. Paste the URL into a custom connector. Match the four-digit code and click Allow on your Mac. If denied or expired, connect again for a new code.")
            if !model.connectorURL.isEmpty { Text(model.connectorURL).font(.caption).textSelection(.enabled) }
        default:
            Text("You're ready to use what you enabled.").font(.title2)
            Text(model.syncEnabled ? "✓ Background sync is on" : "Background sync is off — return to Welcome to enable it.")
            Text("\(SetupPermission.all.filter { model.state.label($0.id, status: model.statuses[$0.id]) == "Allowed" }.count) permissions allowed · \(model.state.skipped.count) skipped")
            Text(model.state.phone ? "✓ iPhone paired" : "iPhone not paired — Mac features still work.")
            ForEach(["Claude.ai", "ChatGPT"], id: \.self) { provider in
                Text(model.state.connectors.contains(provider) ? "✓ \(provider) connected" : "\(provider) not connected — return to Connect AI to add it.")
            }
            Text("Missing access? Open Setup > Open Setup, go back to Permissions, and use Open Settings. Changes appear here automatically.")
        }
    }
    var remoteControls: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(model.remoteReady ? "✓ Remote ready" : "Remote is needed for iPhone pairing and AI connectors.")
            if !model.remoteReady { Button(model.busy.contains("remote") ? "Connecting…" : "Enable Remote") { model.enableRemote() }.disabled(model.busy.contains("remote")) }
        }
    }
}

struct SetupButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.primary)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color(nsColor: .separatorColor)))
            .opacity(enabled ? configuration.isPressed ? 0.7 : 1 : 0.45)
    }
}
