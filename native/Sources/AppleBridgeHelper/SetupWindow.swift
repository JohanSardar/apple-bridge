import AppKit
import SwiftUI
import SourceSupport

struct LocalAgent: Identifiable, Equatable {
    let id: String
    let title: String
    let path: String?
    var added: Bool
}

enum AgentLink: Equatable {
    case idle, waiting, connected, failed
}

@MainActor
final class SetupModel: ObservableObject {
    @Published var state = SetupState()
    @Published var statuses: [String: String] = [:]
    @Published var busy: Set<String> = []
    @Published var declined: Set<String> = []
    @Published var waitingInSettings: String?
    @Published var celebrating: String?
    @Published var agents: [LocalAgent] = []
    @Published var links: [String: AgentLink] = [:]
    @Published var copied = false
    let home: URL
    let permissions = SetupPermissions()
    let testing: Bool
    var guide: SetupGuide?
    var onActivate: () -> Void = {}
    var onFinish: () -> Void = {}
    var refreshTask: Task<Void, Never>?
    var grantedThisRun = false
    var runCommand: @Sendable ([String], String) -> Data? = { arguments, home in SetupModel.command(arguments, home: home) }
    var openURL: (URL) -> Void = { NSWorkspace.shared.open($0) }
    private var icons: [String: NSImage] = [:]

    init(home: URL, testing: Bool = false) {
        self.home = home
        self.testing = testing
        if let data = try? Data(contentsOf: home.appendingPathComponent("setup.json")), let saved = try? JSONDecoder().decode(SetupState.self, from: data) {
            state = saved
        }
    }

    var address: String { "http://127.0.0.1:7780/mcp" }
    var plan: [SetupPermission] { state.plan }
    var current: SetupPermission? { state.current.flatMap(SetupPermission.named) }
    var position: Int { (plan.firstIndex { $0.id == state.current } ?? 0) + 1 }
    func allowed(_ id: String) -> Bool { SetupState.allowed(statuses[id]) }
    var askCount: Int { plan.filter { !allowed($0.id) }.count }

    func save() {
        guard !testing else { return }
        try? FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        guard let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: home.appendingPathComponent("setup.json"), options: .atomic)
    }

    func go(_ step: SetupStep) {
        withAnimation(.easeInOut(duration: 0.25)) { state.step = step }
        if step == .connect { prepareConnect() }
        save()
    }

    func toggle(_ service: SetupService) {
        if state.services.contains(service.id) { state.services.remove(service.id) } else { state.services.insert(service.id) }
        save()
    }

    func beginPermissions() {
        Task { @MainActor in
            await refresh()
            state.skipped = []
            declined = []
            guard let first = state.next(after: nil, statuses: statuses) else { finishPermissions(); return }
            state.current = first.id
            go(.permissions)
        }
    }

    func advance() {
        closeGuide()
        celebrating = nil
        waitingInSettings = nil
        if let next = state.next(after: state.current, statuses: statuses) {
            withAnimation(.easeInOut(duration: 0.25)) { state.current = next.id }
            save()
        } else {
            finishPermissions()
        }
    }

    func back() {
        closeGuide()
        waitingInSettings = nil
        let plan = plan
        if let index = plan.firstIndex(where: { $0.id == state.current }), index > 0 {
            withAnimation(.easeInOut(duration: 0.25)) { state.current = plan[index - 1].id }
            save()
        } else {
            go(.services)
        }
    }

    func skip() {
        if let id = state.current { state.skipped.insert(id) }
        advance()
    }

    func finishPermissions() {
        closeGuide()
        state.current = nil
        go(.connect)
        guard grantedThisRun, !testing else { return }
        let path = home.path
        let run = runCommand
        Task.detached { _ = run(["permissions", "reload"], path) }
    }

    func allow(_ permission: SetupPermission) {
        if permission.usesSettings || declined.contains(permission.id) {
            openSettings(permission)
            return
        }
        guard !testing, busy.insert(permission.id).inserted else { return }
        Task { @MainActor in
            defer { busy.remove(permission.id) }
            do { try await permissions.request(permission) } catch {}
            await refresh()
            if !allowed(permission.id) { withAnimation { _ = declined.insert(permission.id) } }
        }
    }

    func openSettings(_ permission: SetupPermission) {
        waitingInSettings = permission.id
        guard !testing else { return }
        openURL(permission.settingsURL)
        if permission.usesSettings {
            if guide == nil { guide = SetupGuide(model: self) }
            guide?.show()
        }
    }

    func closeGuide() {
        guide?.close()
        guide = nil
    }

    func start() {
        guard refreshTask == nil else { return }
        refreshTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.refresh()
                self.guide?.follow()
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
    }

    func stop() { refreshTask?.cancel(); refreshTask = nil; closeGuide() }

    func refresh() async {
        guard !testing else { return }
        statuses = await permissions.status()
        guard state.step == .permissions, let id = state.current, celebrating == nil, allowed(id) else { return }
        grantedThisRun = true
        withAnimation(.spring(duration: 0.4)) { celebrating = id }
        declined.remove(id)
        let returning = guide != nil
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(returning ? 700 : 900))
            if returning {
                closeGuide()
                onActivate()
                try? await Task.sleep(for: .milliseconds(500))
            }
            guard celebrating == id else { return }
            advance()
        }
    }

    func icon(_ bundle: String?) -> NSImage? {
        guard let bundle else { return nil }
        if let cached = icons[bundle] { return cached }
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { return nil }
        let image = NSWorkspace.shared.icon(forFile: url.path)
        icons[bundle] = image
        return image
    }

    nonisolated static func command(_ arguments: [String], home: String) -> Data? {
        guard let path = try? TrustedChild.bridge() else { return nil }
        return TrustedChild.capture(path, identifier: TrustedChild.bridgeName, arguments: arguments + ["--home", home], timeout: 900)
    }

    static func agentPath() {
        let user = FileManager.default.homeDirectoryForCurrentUser.path
        let extra = ["\(user)/.local/bin", "\(user)/.claude/local", "/opt/homebrew/bin", "/usr/local/bin", "\(user)/.npm-global/bin"]
        let current = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin:/usr/sbin:/sbin"
        setenv("PATH", (extra + [current]).joined(separator: ":"), 1)
    }

    func prepareConnect() {
        let user = FileManager.default.homeDirectoryForCurrentUser
        func find(_ name: String) -> String? {
            let directories = ["\(user.path)/.local/bin", "\(user.path)/.claude/local", "/opt/homebrew/bin", "/usr/local/bin", "\(user.path)/.npm-global/bin"]
            return directories.map { "\($0)/\(name)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
        }
        var claudeAdded = false
        if let data = try? Data(contentsOf: user.appendingPathComponent(".claude.json")),
           let config = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let servers = config["mcpServers"] as? [String: Any] { claudeAdded = servers["apple-bridge"] != nil }
        let codexConfig = (try? String(contentsOf: user.appendingPathComponent(".codex/config.toml"), encoding: .utf8)) ?? ""
        agents = [
            LocalAgent(id: "claude", title: "Claude Code", path: find("claude"), added: claudeAdded),
            LocalAgent(id: "codex", title: "Codex", path: find("codex"), added: codexConfig.contains("[mcp_servers.apple-bridge]"))
        ]
        guard !testing, !FileManager.default.fileExists(atPath: home.appendingPathComponent("run/control.sock").path) else { return }
        let path = home.path
        let run = runCommand
        busy.insert("gateway")
        Task { @MainActor in
            _ = await Task.detached { run(["up"], path) }.value
            busy.remove("gateway")
        }
    }

    func connect(_ agent: LocalAgent) {
        guard !testing, links[agent.id] != .waiting else { return }
        links[agent.id] = .waiting
        let path = home.path
        let run = runCommand
        Task { @MainActor in
            let data = await Task.detached { run(["link", agent.id, "--sign-in"], path) }.value
            withAnimation { links[agent.id] = data == nil ? .failed : .connected }
            if data != nil { onActivate() }
        }
    }

    func copyAddress() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(address, forType: .string)
        withAnimation { copied = true }
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            withAnimation { copied = false }
        }
    }
}

extension SetupTint {
    var color: Color {
        switch self {
        case .blue: .blue
        case .green: .green
        case .orange: .orange
        case .red: .red
        case .yellow: .yellow
        case .purple: .purple
        case .gray: Color(white: 0.45)
        case .indigo: .indigo
        }
    }
}

struct SymbolTile: View {
    let symbol: String
    let tint: Color
    var size: CGFloat = 44
    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
            .fill(LinearGradient(colors: [tint.opacity(0.85), tint], startPoint: .top, endPoint: .bottom))
            .frame(width: size, height: size)
            .overlay(Image(systemName: symbol).font(.system(size: size * 0.48, weight: .semibold)).foregroundStyle(.white))
            .shadow(color: .black.opacity(0.12), radius: size * 0.06, y: size * 0.03)
    }
}

struct AppTile: View {
    @ObservedObject var model: SetupModel
    let service: SetupService
    var size: CGFloat = 44
    var body: some View {
        if let image = model.icon(service.app) {
            Image(nsImage: image).resizable().interpolation(.high).frame(width: size * 1.18, height: size * 1.18).frame(width: size, height: size)
        } else {
            SymbolTile(symbol: service.symbol, tint: service.id == "location" ? .blue : .indigo, size: size * 0.86).frame(width: size, height: size)
        }
    }
}

struct DraggableApp: View {
    var size: CGFloat = 64
    @State private var hovering = false
    var body: some View {
        VStack(spacing: 6) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath))
                .resizable().frame(width: size, height: size)
                .scaleEffect(hovering ? 1.06 : 1)
                .shadow(color: .black.opacity(hovering ? 0.25 : 0.12), radius: hovering ? 8 : 4, y: 2)
            Text("Apple Bridge").font(.caption.weight(.medium))
        }
        .padding(10)
        .contentShape(Rectangle())
        .onHover { inside in withAnimation(.easeOut(duration: 0.15)) { hovering = inside } }
        .onDrag { NSItemProvider(object: Bundle.main.bundleURL as NSURL) }
        .help("Drag into the list in System Settings")
    }
}

struct SetupView: View {
    @ObservedObject var model: SetupModel
    var body: some View {
        VStack(spacing: 0) {
            Group {
                switch model.state.step {
                case .welcome: WelcomeStep(model: model)
                case .services: ServicesStep(model: model)
                case .permissions: PermissionStep(model: model)
                case .connect: ConnectStep(model: model)
                case .done: DoneStep(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .transition(.asymmetric(insertion: .move(edge: .trailing).combined(with: .opacity), removal: .opacity))
            .id(model.state.step.rawValue + (model.state.current ?? ""))
        }
        .frame(width: 760, height: 580)
        .background(
            LinearGradient(colors: [Color(nsColor: .windowBackgroundColor), Color(nsColor: .underPageBackgroundColor).opacity(0.6)], startPoint: .top, endPoint: .bottom)
        )
    }
}

struct StepFrame<Content: View, Footer: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder var content: Content
    @ViewBuilder var footer: Footer
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                Text(title).font(.system(size: 28, weight: .bold))
                Text(subtitle).font(.title3).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.top, 44)
            content.padding(.top, 24)
            Spacer(minLength: 12)
            HStack(spacing: 12) { footer }
                .controlSize(.large)
                .padding(.bottom, 28)
        }
        .padding(.horizontal, 44)
    }
}

struct WelcomeStep: View {
    @ObservedObject var model: SetupModel
    let featured = ["messages", "photos", "calendar", "reminders", "notes", "mail"]
    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            HStack(spacing: 14) {
                ForEach(SetupService.all.filter { featured.contains($0.id) }) { service in
                    AppTile(model: model, service: service, size: 64)
                }
            }
            .padding(.bottom, 30)
            Text("Apple Bridge").font(.system(size: 40, weight: .bold))
            Text("Let the AI agents on this Mac use your Messages, Photos, Calendar and more.")
                .font(.title3).foregroundStyle(.secondary).multilineTextAlignment(.center)
                .frame(maxWidth: 480).padding(.top, 8)
            VStack(alignment: .leading, spacing: 12) {
                Label("Your data stays on this Mac", systemImage: "lock.fill")
                Label("You choose what to connect", systemImage: "checklist")
                Label("Takes about two minutes", systemImage: "clock")
            }
            .font(.body).foregroundStyle(.secondary).labelStyle(WelcomeLabel())
            .padding(.top, 30)
            Spacer()
            Button { model.go(.services) } label: { Text("Get Started").frame(minWidth: 180) }
                .buttonStyle(.borderedProminent).controlSize(.extraLarge).keyboardShortcut(.defaultAction)
                .padding(.bottom, 40)
        }
        .frame(maxWidth: .infinity)
    }
}

struct WelcomeLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 10) {
            configuration.icon.foregroundStyle(Color.accentColor).frame(width: 20)
            configuration.title
        }
    }
}

struct ServicesStep: View {
    @ObservedObject var model: SetupModel
    let columns = Array(repeating: GridItem(.flexible(), spacing: 12), count: 3)
    var summary: String {
        let count = model.askCount
        if model.state.services.isEmpty { return "Choose at least one to continue." }
        return count == 0 ? "Everything these need is already allowed." : count == 1 ? "Apple Bridge will ask for 1 permission." : "Apple Bridge will ask for \(count) permissions, one at a time."
    }
    var body: some View {
        StepFrame(title: "What should your agents reach?", subtitle: "Pick the apps to connect. Setup asks only for what they need, and you can change this later.") {
            LazyVGrid(columns: columns, spacing: 12) {
                ForEach(SetupService.all) { service in ServiceCard(model: model, service: service) }
            }
        } footer: {
            Button("Back") { model.go(.welcome) }
            Text(summary).font(.callout).foregroundStyle(.secondary).padding(.leading, 8)
            Spacer()
            Button { model.beginPermissions() } label: { Text("Continue").frame(minWidth: 110) }
                .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                .disabled(model.state.services.isEmpty)
        }
        .task { await model.refresh() }
    }
}

struct ServiceCard: View {
    @ObservedObject var model: SetupModel
    let service: SetupService
    var selected: Bool { model.state.services.contains(service.id) }
    var body: some View {
        Button { withAnimation(.easeOut(duration: 0.15)) { model.toggle(service) } } label: {
            HStack(spacing: 12) {
                AppTile(model: model, service: service, size: 38)
                VStack(alignment: .leading, spacing: 2) {
                    Text(service.title).font(.headline).foregroundStyle(.primary)
                    Text(service.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .font(.title2).foregroundStyle(selected ? Color.accentColor : Color.secondary.opacity(0.5))
            }
            .padding(.horizontal, 14).frame(height: 66)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(selected ? Color.accentColor.opacity(0.09) : Color(nsColor: .controlBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(selected ? Color.accentColor.opacity(0.7) : Color(nsColor: .separatorColor), lineWidth: selected ? 1.5 : 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct ProgressSegments: View {
    let total: Int
    let current: Int
    var body: some View {
        HStack(spacing: 5) {
            ForEach(0..<max(total, 1), id: \.self) { index in
                Capsule().fill(index < current ? Color.accentColor : Color.secondary.opacity(0.22)).frame(height: 4)
            }
        }
        .frame(width: CGFloat(min(total, 14)) * 26)
    }
}

struct PermissionStep: View {
    @ObservedObject var model: SetupModel
    var body: some View {
        if let permission = model.current {
            let done = model.celebrating == permission.id
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("STEP \(model.position) OF \(model.plan.count)").font(.caption.weight(.semibold)).foregroundStyle(.secondary).tracking(0.6)
                    Spacer()
                    ProgressSegments(total: model.plan.count, current: model.position)
                }
                .padding(.top, 36)
                HStack(alignment: .top, spacing: 22) {
                    ZStack {
                        SymbolTile(symbol: permission.symbol, tint: permission.tint.color, size: 76)
                        if done {
                            Image(systemName: "checkmark.circle.fill").font(.system(size: 34)).foregroundStyle(.white, .green)
                                .background(Circle().fill(.white).padding(3))
                                .offset(x: 32, y: 32).transition(.scale.combined(with: .opacity))
                        }
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        Text(done ? "\(permission.title) is on" : permission.action).font(.system(size: 28, weight: .bold)).contentTransition(.opacity)
                        Text(permission.why).font(.title3).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.top, 30)
                Group {
                    if permission.usesSettings { SettingsInstructions(model: model, permission: permission) }
                    else { PromptInstructions(model: model, permission: permission) }
                }
                .padding(.top, 26)
                Spacer(minLength: 8)
                HStack(spacing: 12) {
                    Button("Back") { model.back() }
                    Spacer()
                    if !done {
                        Button("Skip") { model.skip() }
                        Button { model.allow(permission) } label: {
                            HStack(spacing: 6) {
                                if model.busy.contains(permission.id) { ProgressView().controlSize(.small) }
                                Text(primary(permission))
                            }
                            .frame(minWidth: 150)
                        }
                        .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                        .disabled(model.busy.contains(permission.id))
                    }
                }
                .controlSize(.large)
                .padding(.bottom, 28)
            }
            .padding(.horizontal, 44)
        }
    }

    func primary(_ permission: SetupPermission) -> String {
        if model.busy.contains(permission.id) { return "Waiting for macOS…" }
        if permission.usesSettings || model.declined.contains(permission.id) {
            return model.waitingInSettings == permission.id ? "Open System Settings Again" : "Open System Settings"
        }
        return permission.action
    }
}

struct SettingsInstructions: View {
    @ObservedObject var model: SetupModel
    let permission: SetupPermission
    var body: some View {
        let waiting = model.waitingInSettings == permission.id
        let done = model.celebrating == permission.id
        HStack(spacing: 20) {
            DraggableApp(size: 60)
            Image(systemName: "arrow.right").font(.title2.weight(.semibold)).foregroundStyle(.tertiary)
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text(permission.title).font(.subheadline.weight(.semibold))
                    Spacer()
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
                Divider()
                HStack(spacing: 10) {
                    Image(nsImage: NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath)).resizable().frame(width: 22, height: 22)
                    Text("Apple Bridge").font(.subheadline)
                    Spacer()
                    Toggle("", isOn: .constant(done)).toggleStyle(.switch).labelsHidden().controlSize(.small).allowsHitTesting(false)
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
            }
            .frame(width: 260)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color(nsColor: .separatorColor)))
            VStack(alignment: .leading, spacing: 8) {
                Instruction(number: 1, text: "Open System Settings", done: waiting || done)
                Instruction(number: 2, text: "Drag Apple Bridge into the list", done: done)
                Instruction(number: 3, text: "Turn on its switch", done: done)
            }
            .font(.callout)
        }
        StatusLine(model: model, permission: permission, waitingText: "Waiting for you to turn on \(permission.title)…")
            .padding(.top, 18)
    }
}

struct Instruction: View {
    let number: Int
    let text: String
    let done: Bool
    var body: some View {
        HStack(spacing: 8) {
            ZStack {
                Circle().fill(done ? Color.green : Color.secondary.opacity(0.18)).frame(width: 20, height: 20)
                if done { Image(systemName: "checkmark").font(.caption2.weight(.bold)).foregroundStyle(.white) }
                else { Text("\(number)").font(.caption.weight(.semibold)).foregroundStyle(.secondary) }
            }
            Text(text).foregroundStyle(done ? .secondary : .primary)
        }
    }
}

struct PromptInstructions: View {
    @ObservedObject var model: SetupModel
    let permission: SetupPermission
    var hint: String {
        if model.declined.contains(permission.id) {
            return "macOS didn't allow it. Turn on Apple Bridge in System Settings, or skip it for now. Everything else still works."
        }
        if let bundle = permission.bundle, let name = SetupService.all.first(where: { $0.app == bundle })?.title {
            return "macOS will ask whether Apple Bridge may use \(name). Click Allow."
        }
        return "macOS will ask you once. Click Allow."
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: model.declined.contains(permission.id) ? "exclamationmark.triangle.fill" : "hand.tap.fill")
                    .foregroundStyle(model.declined.contains(permission.id) ? Color.orange : Color.accentColor).font(.title3)
                Text(hint).font(.body).fixedSize(horizontal: false, vertical: true)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color(nsColor: .separatorColor)))
            StatusLine(model: model, permission: permission, waitingText: model.declined.contains(permission.id) ? "Waiting for you to turn it on in System Settings…" : "Waiting for your answer…")
        }
    }
}

struct StatusLine: View {
    @ObservedObject var model: SetupModel
    let permission: SetupPermission
    let waitingText: String
    var body: some View {
        if model.celebrating == permission.id {
            Label("Allowed. Moving on…", systemImage: "checkmark.circle.fill").foregroundStyle(.green).font(.headline)
                .transition(.opacity)
        } else if model.waitingInSettings == permission.id || model.busy.contains(permission.id) {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text(waitingText).foregroundStyle(.secondary)
            }
            .transition(.opacity)
        }
    }
}

struct ConnectStep: View {
    @ObservedObject var model: SetupModel
    var body: some View {
        StepFrame(title: "Connect your agent", subtitle: "Agents on this Mac reach Apple Bridge at one local address. It doesn't go through the internet.") {
            VStack(spacing: 10) {
                ForEach(model.agents) { agent in AgentRow(model: model, agent: agent) }
                HStack(spacing: 14) {
                    SymbolTile(symbol: "link", tint: .gray, size: 38)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Any other app").font(.headline)
                        Text(model.address).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                    Spacer()
                    Button(model.copied ? "Copied" : "Copy Address") { model.copyAddress() }
                }
                .padding(14)
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
                .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Color(nsColor: .separatorColor)))
                Text("Connecting opens your browser once. Check that the code there matches the one Apple Bridge shows, then click Allow.")
                    .font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 6)
            }
        } footer: {
            Button("Back") {
                if model.plan.isEmpty { model.go(.services) } else { model.beginPermissions() }
            }
            Spacer()
            Button { model.go(.done) } label: { Text(model.links.values.contains(.connected) ? "Continue" : "Skip for Now").frame(minWidth: 110) }
                .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
        }
    }
}

struct AgentRow: View {
    @ObservedObject var model: SetupModel
    let agent: LocalAgent
    var body: some View {
        let link = model.links[agent.id] ?? .idle
        HStack(spacing: 14) {
            SymbolTile(symbol: "terminal.fill", tint: agent.id == "claude" ? Color(red: 0.85, green: 0.47, blue: 0.34) : .black, size: 38)
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.title).font(.headline)
                Text(detail(link)).font(.callout).foregroundStyle(link == .failed ? Color.orange : .secondary)
            }
            Spacer()
            switch link {
            case .connected:
                Label("Connected", systemImage: "checkmark.circle.fill").foregroundStyle(.green).font(.headline)
            case .waiting:
                HStack(spacing: 6) { ProgressView().controlSize(.small); Text("Waiting for you in the browser…").foregroundStyle(.secondary) }
            case .idle, .failed:
                if agent.path != nil {
                    Button(link == .failed ? "Try Again" : agent.added ? "Connect Again" : "Connect") { model.connect(agent) }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(link == .connected ? Color.green.opacity(0.6) : Color(nsColor: .separatorColor)))
    }

    func detail(_ link: AgentLink) -> String {
        if agent.path == nil { return "Not installed on this Mac" }
        switch link {
        case .connected: return "Ready to use Apple Bridge"
        case .failed: return "Sign-in didn't finish. Try again."
        case .waiting: return "Check the code in the browser, then click Allow"
        case .idle: return agent.added ? "Already added. Connect again to sign in." : "Found on this Mac"
        }
    }
}

struct DoneStep: View {
    @ObservedObject var model: SetupModel
    var connected: [String] {
        model.agents.filter { model.links[$0.id] == .connected || $0.added }.map(\.title)
    }
    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            Image(systemName: "checkmark.circle.fill").font(.system(size: 64)).foregroundStyle(.white, .green).padding(.bottom, 18)
            Text("You're all set").font(.system(size: 34, weight: .bold))
            Text(connected.isEmpty ? "Connect an agent any time with bridge setup, then ask it something like:" : "Open \(connected.joined(separator: " or ")) and ask something like:")
                .font(.title3).foregroundStyle(.secondary).multilineTextAlignment(.center).padding(.top, 8).frame(maxWidth: 520)
            VStack(spacing: 10) {
                ForEach(examples, id: \.self) { example in
                    Text(example).font(.title3)
                        .padding(.horizontal, 18).padding(.vertical, 10)
                        .background(Capsule().fill(Color.accentColor.opacity(0.1)))
                }
            }
            .padding(.top, 24)
            HStack(spacing: 8) {
                ForEach(SetupService.all.filter { model.state.services.contains($0.id) }) { service in
                    let ready = service.permissions.allSatisfy { model.allowed($0) }
                    HStack(spacing: 6) {
                        AppTile(model: model, service: service, size: 20)
                        Image(systemName: ready ? "checkmark" : "minus").font(.caption.weight(.bold)).foregroundStyle(ready ? Color.green : Color.secondary)
                    }
                    .help(ready ? "\(service.title) is ready" : "\(service.title) is missing a permission")
                }
            }
            .padding(.top, 30)
            Spacer()
            Button { model.onFinish() } label: { Text("Done").frame(minWidth: 160) }
                .buttonStyle(.borderedProminent).controlSize(.extraLarge).keyboardShortcut(.defaultAction)
                .padding(.bottom, 40)
        }
        .frame(maxWidth: .infinity)
    }

    var examples: [String] {
        var list: [String] = []
        if model.state.services.contains("calendar") { list.append("“What's on my calendar this week?”") }
        if model.state.services.contains("reminders") { list.append("“Add milk to my shopping list”") }
        if model.state.services.contains("notes") { list.append("“Summarize my latest notes”") }
        if model.state.services.contains("messages") { list.append("“Who texted me today?”") }
        return Array((list.isEmpty ? ["“What can you see on my Mac?”"] : list).prefix(3))
    }
}
