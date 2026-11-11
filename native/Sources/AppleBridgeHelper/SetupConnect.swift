import AppKit
import Foundation
import SwiftUI

struct RemoteRoute: Decodable, Equatable, Sendable {
    let state: String
    let url: String?
    let message: String?
    let link: String?
    let last_error: String?
    let sources: [String]?
    var on: Bool { state == "on" }
    var mcp: String { (url ?? "").isEmpty ? "" : (url ?? "") + "/mcp" }
}

struct CloudService: Identifiable, Hashable, Sendable {
    let id: String
    let title: String
    static let all: [Self] = [
        .init(id: "calendar", title: "Calendar"),
        .init(id: "reminders", title: "Reminders"),
        .init(id: "notes", title: "Notes"),
        .init(id: "contacts", title: "Contacts"),
        .init(id: "messages", title: "Messages"),
        .init(id: "mail", title: "Mail"),
        .init(id: "photos", title: "Photos"),
        .init(id: "files", title: "Files"),
        .init(id: "safari", title: "Safari"),
        .init(id: "usage", title: "Screen Time"),
        .init(id: "location", title: "Location"),
        .init(id: "maps", title: "Maps")
    ]
}

enum DeviceKind: String, CaseIterable, Identifiable {
    case grok = "Grok Bot"
    case claudeCode = "Claude Code"
    case codex = "Codex"
    case other = "Other app"
    var id: String { rawValue }
}

@MainActor
final class ConnectModel: ObservableObject {
    @Published var tailnet: RemoteRoute?
    @Published var cloud: RemoteRoute?
    @Published var relay: RemoteRoute?
    @Published var cloudSources: Set<String> = ["calendar", "reminders", "notes"]
    @Published var device: DeviceKind = .grok
    @Published var deviceKey = ""
    @Published var relayURL = ""
    @Published var relayKey = ""
    @Published var busy: Set<String> = []
    @Published var notice = ""
    @Published var copied = ""
    @Published var tailscaleInstalled = false
    let home: URL
    let testing: Bool
    var run: @Sendable ([String], String) -> Data?
    var openURL: (URL) -> Void = { NSWorkspace.shared.open($0) }
    var copy: (String) -> Void = { value in
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
    }

    init(home: URL, testing: Bool, run: @escaping @Sendable ([String], String) -> Data?) {
        self.home = home
        self.testing = testing
        self.run = run
    }

    static let tailscaleApp = URL(fileURLWithPath: "/Applications/Tailscale.app")
    static let tailscalePackage = URL(string: "https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg")!
    nonisolated static let tailscaleTeam = "W5364U7YZB"
    static let selfHostGuide = URL(string: "https://github.com/JohanSardar/apple-bridge/blob/main/docs/self-host.md")!

    func refresh() async {
        tailscaleInstalled = FileManager.default.fileExists(atPath: Self.tailscaleApp.path)
        guard !testing else { return }
        let path = home.path
        let run = run
        guard let data = await Task.detached(operation: { run(["remote", "status", "--json"], path) }).value else { return }
        struct Envelope: Decodable {
            struct Remote: Decodable { let routes: [String: RemoteRoute]? }
            let remote: Remote
        }
        let routes = (try? JSONDecoder().decode(Envelope.self, from: data))?.remote.routes ?? [:]
        tailnet = routes["tailnet"]
        cloud = routes["system"]
        relay = routes["relay"]
        if let sources = cloud?.sources, !sources.isEmpty, busy.isEmpty { cloudSources = Set(sources) }
    }

    func copyValue(_ value: String, label: String) {
        copy(value)
        copied = label
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            if copied == label { copied = "" }
        }
    }

    func perform(_ key: String, _ arguments: [String], done: @escaping @MainActor (Data?) -> Void = { _ in }) {
        guard !testing, busy.insert(key).inserted else { return }
        let path = home.path
        let run = run
        Task { @MainActor in
            let data = await Task.detached { run(arguments, path) }.value
            await refresh()
            busy.remove(key)
            done(data)
        }
    }

    func turnOnTailnet() {
        notice = ""
        perform("tailnet", ["remote", "enable", "--via", "tailnet", "--json", "--no-open", "--timeout", "45s"]) { [weak self] data in
            if data == nil, let state = self?.tailnet?.state, state != "off" { return }
            self?.openPending(data, fallback: "Couldn't turn on Tailscale access. Make sure Apple Bridge is running (run bridge up) and Tailscale is open, then try again.")
        }
    }

    func turnOffTailnet() {
        deviceKey = ""
        perform("tailnet-off", ["remote", "disable", "--via", "tailnet", "--json", "--timeout", "45s"])
    }

    func turnOnCloud() {
        notice = ""
        guard !cloudSources.isEmpty else { notice = "Choose at least one service for cloud chats."; return }
        let sources = CloudService.all.map(\.id).filter { cloudSources.contains($0) }.joined(separator: ",")
        perform("cloud", ["remote", "enable", "--via", "cloud", "--sources", sources, "--json", "--no-open", "--timeout", "45s"]) { [weak self] data in
            if data == nil, let state = self?.cloud?.state, state != "off" { return }
            self?.openPending(data, fallback: "Couldn't turn on the public address. Make sure Apple Bridge is running and Tailscale is signed in, then try again.")
        }
    }

    func disconnectCloud() {
        perform("cloud-off", ["remote", "disable", "--via", "cloud", "--json", "--timeout", "45s"]) { [weak self] data in
            self?.notice = data == nil ? "Couldn't close the public address. Try again." : "Cloud chats are disconnected and the public address is closed."
        }
    }

    func openPending(_ data: Data?, fallback: String) {
        guard let data else { notice = fallback; return }
        struct Pending: Decodable {
            struct Remote: Decodable { let signInUrl: String? }
            let remote: Remote?
        }
        if let link = (try? JSONDecoder().decode(Pending.self, from: data))?.remote?.signInUrl, let url = URL(string: link), url.scheme == "https" {
            openURL(url)
        }
    }

    func createDeviceKey() {
        let scopes = CloudService.all.map { "\($0.id):write" }.joined(separator: ",")
        perform("key", ["agents", "create", "--name", device == .grok ? "Grok Bot" : device.rawValue, "--tailnet", "--scope", scopes, "--expires-in", "90d", "--json"]) { [weak self] data in
            struct Created: Decodable { let token: String }
            guard let data, let token = (try? JSONDecoder().decode(Created.self, from: data))?.token else {
                self?.notice = "Couldn't make a key. Turn on Tailscale access first."
                return
            }
            self?.deviceKey = token
        }
    }

    func installTailscale() {
        guard !testing, busy.insert("install").inserted else { return }
        notice = "Downloading Tailscale from tailscale.com…"
        Task { @MainActor in
            defer { busy.remove("install") }
            do {
                let (file, _) = try await URLSession.shared.download(from: Self.tailscalePackage)
                let package = FileManager.default.temporaryDirectory.appendingPathComponent("Tailscale-\(UUID().uuidString).pkg")
                try FileManager.default.moveItem(at: file, to: package)
                guard await Task.detached(operation: { Self.signedByTailscale(package) }).value else {
                    try? FileManager.default.removeItem(at: package)
                    notice = "The download wasn't signed by Tailscale, so it wasn't opened. Get Tailscale from tailscale.com/download instead."
                    return
                }
                openURL(package)
                notice = "Follow the installer. When it finishes, open Tailscale from Applications and sign in, then come back here."
            } catch {
                notice = "Couldn't download Tailscale. Check your internet connection, or get it from tailscale.com/download."
            }
        }
    }

    nonisolated static func signedByTailscale(_ package: URL) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/pkgutil")
        process.arguments = ["--check-signature", package.path]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        guard (try? process.run()) != nil else { return false }
        let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        process.waitUntilExit()
        return process.terminationStatus == 0 && output.contains("Developer ID Installer: Tailscale Inc. (\(tailscaleTeam))") && output.contains("Notarization: trusted by the Apple notary service")
    }

    func showRelayKey() {
        let url = relayURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard url.hasPrefix("https://") else { notice = "Enter your relay's address, starting with https://."; return }
        perform("relay-key", ["remote", "key", "--relay-url", url, "--json"]) { [weak self] data in
            struct Key: Decodable { let key: String }
            self?.relayKey = data.flatMap { try? JSONDecoder().decode(Key.self, from: $0) }?.key ?? ""
            if self?.relayKey.isEmpty == true { self?.notice = "Couldn't read this Mac's relay key." }
        }
    }

    func connectRelay() {
        let url = relayURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard url.hasPrefix("https://") else { notice = "Enter your relay's address, starting with https://."; return }
        perform("relay", ["remote", "enable", "--via", "relay", "--relay-url", url, "--json", "--no-open", "--timeout", "45s"]) { [weak self] data in
            self?.openPending(data, fallback: "Couldn't reach your relay. Check its address and that it lists this Mac's key.")
        }
    }

    func disconnectRelay() {
        perform("relay", ["remote", "disable", "--via", "relay", "--json", "--timeout", "45s"])
    }
}

struct ConnectView: View {
    @ObservedObject var model: ConnectModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Apple Bridge already works with agents on this Mac. Add other devices and cloud chats here. Everything below is optional.")
                .foregroundStyle(.secondary)
            if !model.notice.isEmpty {
                Label(model.notice, systemImage: "info.circle.fill")
                    .foregroundStyle(.primary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
                    .textSelection(.enabled)
            }
            localCard
            tailnetCard
            cloudCard
            relayCard
        }
    }

    var localCard: some View {
        ConnectCard(symbol: "laptopcomputer", title: "On this Mac", badge: "Ready", badgeColor: .green) {
            Text("Claude Code, Codex and other agents running on this Mac connect to the local address. Nothing leaves this Mac.")
                .foregroundStyle(.secondary)
            AddressRow(model: model, label: "Local address", value: "http://127.0.0.1:7780/mcp")
        }
    }

    var tailnetCard: some View {
        ConnectCard(symbol: "lock.shield", title: "Your other devices and Grok Bot", badge: model.tailnet?.on == true ? "On" : "Recommended", badgeColor: model.tailnet?.on == true ? .green : .accentColor) {
            Text("Uses Tailscale, a free private network. Only devices signed in to your Tailscale account can reach this Mac. Nothing is on the public internet.")
                .foregroundStyle(.secondary)
            if !model.tailscaleInstalled && model.tailnet?.on != true {
                StepRow(number: 1, text: "Install Tailscale on this Mac. It's free and takes about a minute.") {
                    Button(model.busy.contains("install") ? "Downloading…" : "Install Tailscale") { model.installTailscale() }
                        .disabled(model.busy.contains("install"))
                }
            }
            if let route = model.tailnet, route.on {
                AddressRow(model: model, label: "Tailscale address", value: route.mcp)
                Picker("Connect", selection: $model.device) {
                    ForEach(DeviceKind.allCases) { Text($0.rawValue).tag($0) }
                }.pickerStyle(.segmented).labelsHidden()
                deviceSteps(route)
                HStack {
                    Spacer()
                    Button(model.busy.contains("tailnet-off") ? "Turning Off…" : "Turn Off") { model.turnOffTailnet() }.disabled(model.busy.contains("tailnet-off"))
                }
            } else if let route = model.tailnet, route.state != "off" {
                pendingNotice(route)
                HStack { Spacer(); Button("Cancel") { model.turnOffTailnet() }.disabled(model.busy.contains("tailnet-off")) }
            } else {
                StepRow(number: model.tailscaleInstalled ? 1 : 2, text: "Turn on Tailscale access for Apple Bridge. If Tailscale asks you to sign in, use the account your other devices use.") {
                    Button(model.busy.contains("tailnet") ? "Turning On…" : "Turn On") { model.turnOnTailnet() }
                        .buttonStyle(PrimaryButtonStyle())
                        .disabled(model.busy.contains("tailnet") || !model.tailscaleInstalled)
                }
            }
        }
    }

    @ViewBuilder func deviceSteps(_ route: RemoteRoute) -> some View {
        switch model.device {
        case .grok:
            StepRow(number: 1, text: "In Grok Bot, open Connect apps, choose Tailscale and click Authenticate. Sign in with the same Tailscale account.") { EmptyView() }
            StepRow(number: 2, text: "Make a key for Grok Bot. It works only on your Tailscale network and expires in 90 days.") {
                Button(model.busy.contains("key") ? "Making…" : "Make Key") { model.createDeviceKey() }.disabled(model.busy.contains("key"))
            }
            if !model.deviceKey.isEmpty {
                StepRow(number: 3, text: "In Grok Bot, add a custom MCP server with this address and header.") { EmptyView() }
                AddressRow(model: model, label: "Address", value: route.mcp)
                AddressRow(model: model, label: "Header", value: "Authorization: Bearer \(model.deviceKey)", display: "Authorization: Bearer ••••••••", copyLabel: "Header")
                Text("The key is shown only here. Copy it now; you can make a new one any time.").font(.caption).foregroundStyle(.secondary)
            }
        case .claudeCode:
            StepRow(number: 1, text: "Install Tailscale on the other computer and sign in with the same account.") { EmptyView() }
            StepRow(number: 2, text: "On that computer, run this in Terminal, then type /mcp in Claude Code to sign in.") { EmptyView() }
            AddressRow(model: model, label: "Command", value: "claude mcp add --transport http apple-bridge \(route.mcp)", copyLabel: "Command")
            StepRow(number: 3, text: "A window on this Mac shows a four-digit code. Click Allow if it matches.") { EmptyView() }
        case .codex:
            StepRow(number: 1, text: "Install Tailscale on the other computer and sign in with the same account.") { EmptyView() }
            StepRow(number: 2, text: "On that computer, run this in Terminal. Codex opens a sign-in page.") { EmptyView() }
            AddressRow(model: model, label: "Command", value: "codex mcp add apple-bridge --url \(route.mcp) && codex mcp login apple-bridge", copyLabel: "Command")
            StepRow(number: 3, text: "A window on this Mac shows a four-digit code. Click Allow if it matches.") { EmptyView() }
        case .other:
            StepRow(number: 1, text: "Install Tailscale on that device and sign in with the same account.") { EmptyView() }
            StepRow(number: 2, text: "Add a remote MCP server in the app with this address. When it asks you to sign in, approve on this Mac. Apps that only take a fixed header can use a key instead.") {
                Button(model.busy.contains("key") ? "Making…" : "Make Key") { model.createDeviceKey() }.disabled(model.busy.contains("key"))
            }
            AddressRow(model: model, label: "Address", value: route.mcp)
            if !model.deviceKey.isEmpty {
                AddressRow(model: model, label: "Header", value: "Authorization: Bearer \(model.deviceKey)", display: "Authorization: Bearer ••••••••", copyLabel: "Header")
            }
        }
    }

    @ViewBuilder func pendingNotice(_ route: RemoteRoute?) -> some View {
        if let route, !route.on, route.state != "off" {
            let action: String? = switch route.state {
            case "login_pending": "Sign In to Tailscale"
            case "https_pending": "Turn On HTTPS"
            case "funnel_pending": "Allow Funnel"
            default: nil
            }
            let text = switch route.state {
            case "login_pending": "Tailscale needs you to sign in."
            case "https_pending": "Tailscale needs HTTPS turned on for your network, once. Click Turn On HTTPS, then Enable HTTPS."
            case "funnel_pending": "Tailscale needs you to allow Funnel for your network, once. Click Allow Funnel and confirm."
            case "error": route.last_error ?? "Something went wrong. Turn it off and try again."
            default: "Starting…"
            }
            HStack(alignment: .firstTextBaseline) {
                Label(text, systemImage: route.state == "error" ? "exclamationmark.triangle" : "hourglass").foregroundStyle(route.state == "error" ? .orange : .secondary)
                Spacer()
                if let action, let link = route.link, let url = URL(string: link) {
                    Button(action) { model.openURL(url) }.buttonStyle(PrimaryButtonStyle())
                }
            }
        }
    }

    var cloudCard: some View {
        ConnectCard(symbol: "globe", title: "Cloud chats: Claude and ChatGPT", badge: model.cloud?.on == true ? "On" : "Off", badgeColor: model.cloud?.on == true ? .orange : .secondary) {
            Text("Claude and ChatGPT run in the cloud and can't join Tailscale. To use them, your Mac gets a public web address through Tailscale Funnel. Only chats you approve on this Mac get in, but the address can be reached from anywhere, so this is less private than Tailscale between your devices.")
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let route = model.cloud, route.on {
                AddressRow(model: model, label: "Connector address", value: route.mcp)
                Text("A new public address can take up to a minute to start answering.").font(.caption).foregroundStyle(.secondary)
                StepRow(number: 1, text: "Claude: click Open Claude. In Add custom connector, name it Apple Bridge, paste the address (it's copied), click Add, then Connect.") {
                    Button("Open Claude") { model.copyValue(route.mcp, label: "Connector address"); model.openURL(URL(string: "https://claude.ai/new?modal=add-custom-connector#customize/connectors")!) }
                }
                StepRow(number: 2, text: "ChatGPT: Settings → Apps → Advanced → Developer mode, then Create app. Paste the address and choose OAuth.") {
                    Button("Open ChatGPT") { model.copyValue(route.mcp, label: "Connector address"); model.openURL(URL(string: "https://chatgpt.com/#settings/Connectors")!) }
                }
                StepRow(number: 3, text: "A window on this Mac shows the chat's name, what it can see, and a four-digit code. Click Allow if it matches the code in your browser.") { EmptyView() }
                HStack {
                    Text("Sharing: " + CloudService.all.filter { model.cloudSources.contains($0.id) }.map(\.title).joined(separator: ", ")).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button(model.busy.contains("cloud-off") ? "Disconnecting…" : "Disconnect and Close Address") { model.disconnectCloud() }
                        .buttonStyle(DestructiveButtonStyle())
                        .disabled(model.busy.contains("cloud-off"))
                }
            } else {
                Text("Choose what cloud chats may see").font(.headline)
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: 4), alignment: .leading, spacing: 6) {
                    ForEach(CloudService.all) { service in
                        Toggle(service.title, isOn: Binding(
                            get: { model.cloudSources.contains(service.id) },
                            set: { if $0 { model.cloudSources.insert(service.id) } else { model.cloudSources.remove(service.id) } }
                        )).toggleStyle(.checkbox)
                    }
                }
                pendingNotice(model.cloud)
                HStack {
                    if !model.tailscaleInstalled { Text("Install Tailscale first, above.").font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    if model.cloud.map({ $0.state != "off" }) == true {
                        Button("Cancel") { model.disconnectCloud() }.disabled(model.busy.contains("cloud-off"))
                    } else {
                        Button(model.busy.contains("cloud") ? "Turning On…" : "Turn On Public Address") { model.turnOnCloud() }
                            .buttonStyle(PrimaryButtonStyle())
                            .disabled(model.busy.contains("cloud") || !model.tailscaleInstalled)
                    }
                }
            }
        }
    }

    var relayCard: some View {
        ConnectCard(symbol: "server.rack", title: "Your own relay", badge: model.relay?.on == true ? "On" : "Advanced", badgeColor: model.relay?.on == true ? .green : .secondary) {
            Text("If you run your own server, it can give this Mac a public address instead of Tailscale. It forwards encrypted traffic; the keys stay on this Mac.")
                .foregroundStyle(.secondary)
            if let route = model.relay, route.on {
                AddressRow(model: model, label: "Relay address", value: route.mcp)
                HStack { Spacer(); Button("Turn Off") { model.disconnectRelay() } }
            } else {
                HStack {
                    TextField("https://bridge.example.com", text: $model.relayURL).textFieldStyle(.roundedBorder)
                    Button("Show This Mac's Key") { model.showRelayKey() }.disabled(model.busy.contains("relay-key"))
                    Button(model.busy.contains("relay") ? "Connecting…" : "Connect") { model.connectRelay() }.disabled(model.busy.contains("relay"))
                }
                if !model.relayKey.isEmpty { AddressRow(model: model, label: "Key for your relay", value: model.relayKey, copyLabel: "Key") }
                pendingNotice(model.relay)
                Button("How to set up a relay") { model.openURL(ConnectModel.selfHostGuide) }.buttonStyle(.link)
            }
        }
    }
}

struct ConnectCard<Content: View>: View {
    let symbol: String
    let title: String
    let badge: String
    let badgeColor: Color
    @ViewBuilder let content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: symbol).font(.title3).foregroundStyle(Color.accentColor).frame(width: 26)
                Text(title).font(.title3.weight(.semibold))
                Spacer()
                Text(badge).font(.caption.weight(.semibold)).foregroundStyle(badgeColor)
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .background(badgeColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 5))
            }
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color(nsColor: .separatorColor)))
    }
}

struct StepRow<Accessory: View>: View {
    let number: Int
    let text: String
    @ViewBuilder let accessory: Accessory
    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Text("\(number)").font(.caption.weight(.bold)).foregroundStyle(.white)
                .frame(width: 20, height: 20).background(Color.accentColor, in: Circle())
            Text(text).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 8)
            accessory
        }
    }
}

struct AddressRow: View {
    @ObservedObject var model: ConnectModel
    let label: String
    let value: String
    var display: String?
    var copyLabel: String?
    var body: some View {
        HStack(spacing: 10) {
            Text(label).font(.caption).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
            Text(display ?? value).font(.system(.body, design: .monospaced)).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                .padding(.horizontal, 8).padding(.vertical, 5)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
            Button(model.copied == (copyLabel ?? label) ? "Copied" : "Copy") { model.copyValue(value, label: copyLabel ?? label) }
                .frame(minWidth: 60)
        }
    }
}

struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 6))
            .opacity(enabled ? configuration.isPressed ? 0.75 : 1 : 0.45)
    }
}

struct DestructiveButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.red)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.red.opacity(0.6)))
            .opacity(enabled ? configuration.isPressed ? 0.7 : 1 : 0.45)
    }
}
