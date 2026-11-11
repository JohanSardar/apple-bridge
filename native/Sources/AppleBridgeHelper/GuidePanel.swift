import AppKit
import AppleSession
import Contacts
import EventKit
import MessagesSource
import QuartzCore
import SwiftUI

@MainActor
final class GuideController: ObservableObject {
    @Published var flow: GuideFlow
    @Published var dragging = false
    let panel = FloatingPanel(size: NSSize(width: 300, height: 392))
    let ghost = GhostOverlay()
    var ticker: Task<Void, Never>?
    var settingsFrame: CGRect?
    var passwordUp = false
    var openedPane = ""
    var askedPrompt = ""
    var lastGhostRoute: (CGPoint, CGPoint)?
    var paneOpenedAt: Date?
    var paneReopened = false
    var allowedAt: Date?
    @Published var sheetUp = false

    static var resumeFile: URL { PrivateReport.directory.appendingPathComponent("permission-guide.json") }
    static var appIcon: NSImage { NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath) }

    init(flow: GuideFlow) {
        self.flow = flow
    }

    static func start(_ arguments: [String]) {
        var steps: [String] = []
        var report = ""
        var index = 1
        while index + 1 < arguments.count {
            if arguments[index] == "--steps" { steps = arguments[index + 1].split(separator: ",").map(String.init) }
            if arguments[index] == "--report", let url = try? PrivateReport.validate(arguments[index + 1]) { report = url.path }
            index += 2
        }
        let flow = arguments.first == "permission-guide" ? GuideFlow(steps: GuideStep.steps(steps), report: report) : (GuideFlow.resumable(try? Data(contentsOf: resumeFile)) ?? GuideFlow(steps: GuideStep.steps([])))
        let controller = GuideController(flow: flow)
        retained = controller
        controller.begin()
    }

    static var retained: GuideController?

    func begin() {
        panel.contentView = NSHostingView(rootView: GuideView(controller: self))
        Task { @MainActor in
            flow.dropAlreadyAllowed(await granted(Set(flow.steps.map(\.id))))
            save()
            place()
            panel.orderFrontRegardless()
            ticker = Task { @MainActor [weak self] in
                while !Task.isCancelled {
                    await self?.tick()
                    try? await Task.sleep(for: .milliseconds(450))
                }
            }
        }
    }

    func tick() async {
        guard let step = flow.current else { return finish() }
        if step.kind == .drag {
            await dragTick(step)
        } else if flow.moment == .asking, askedPrompt != step.id {
            askedPrompt = step.id
            ghost.hide()
            place()
            try? await Task.sleep(for: .milliseconds(700))
            let outcome = try? await NewPermissions.request(step.id)
            let allowed = await granted([step.id]).contains(step.id)
            if allowed { flow.allow() } else if outcome == .timedOut { flow.skip(); save(); return } else { flow.decline() }
            await advance(after: allowed ? 1.1 : 1.8)
        }
        place()
    }

    func dragTick(_ step: GuideStep) async {
        let dialog = ScreenGeometry.dialogShowing
        sheetUp = dialog
        if openedPane != step.id, !dialog, let url = step.settingsURL {
            openedPane = step.id
            paneReopened = false
            paneOpenedAt = Date()
            NSWorkspace.shared.open(url)
        } else if !paneReopened, !dialog, let opened = paneOpenedAt, Date().timeIntervalSince(opened) > 1.6, ScreenGeometry.settingsWindow != nil, let url = step.settingsURL {
            paneReopened = true
            NSWorkspace.shared.open(url)
        }
        if dialog && !passwordUp && flow.moment != .allowed {
            passwordUp = true
            flow.dropped()
            handOverKeyboard()
        } else if !dialog && passwordUp {
            passwordUp = false
            let allowed = await granted([step.id]).contains(step.id)
            flow.passwordClosed(granted: allowed)
            save()
        }
        if [.waitingForDrop, .cancelledPassword, .confirming].contains(flow.moment), await granted([step.id]).contains(step.id) {
            flow.allow()
            save()
        }
        guard flow.moment == .allowed else { return }
        if allowedAt == nil { allowedAt = Date() }
        let waited = Date().timeIntervalSince(allowedAt ?? Date())
        if dialog && waited < 120 { return }
        if waited < 1.4 { return }
        passwordUp = false
        allowedAt = nil
        await advance(after: 0.6)
    }

    func advance(after seconds: Double) async {
        save()
        try? await Task.sleep(for: .seconds(seconds))
        guard flow.moment == .allowed || flow.moment == .declined else { return }
        withAnimation(.easeInOut(duration: 0.25)) { flow.next() }
        save()
    }

    func notNow() {
        withAnimation(.easeInOut(duration: 0.25)) { flow.skip() }
        save()
    }

    func dropped() {
        flow.dropped()
        save()
    }

    func handOverKeyboard() {
        ghost.hide()
        for identifier in ["com.apple.SecurityAgent", "com.apple.coreautha"] {
            NSRunningApplication.runningApplications(withBundleIdentifier: identifier).first?.activate()
        }
    }

    func place() {
        guard !flow.finished || !panel.isVisible else { return ghost.hide() }
        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        let size = panel.frame.size
        settingsFrame = flow.current?.kind == .drag ? ScreenGeometry.settingsWindow : nil
        var origin: CGPoint
        if let settings = settingsFrame {
            let sidebarEdge = settings.minX + min(232, settings.width * 0.32)
            let beside = settings.minX - size.width - 16
            if beside >= screen.minX + 8 {
                origin = CGPoint(x: beside, y: settings.maxY - size.height - 40)
            } else {
                origin = CGPoint(x: max(screen.minX + 8, sidebarEdge - size.width), y: settings.maxY - size.height - 48)
            }
        } else if flow.current?.kind == .prompt {
            origin = CGPoint(x: screen.midX - size.width / 2, y: screen.maxY - size.height - min(470, screen.height * 0.52))
        } else {
            origin = CGPoint(x: screen.midX - size.width / 2, y: screen.midY - size.height / 2 + screen.height * 0.08)
        }
        origin.y = max(screen.minY + 8, origin.y)
        if abs(panel.frame.origin.x - origin.x) > 1 || abs(panel.frame.origin.y - origin.y) > 1 {
            panel.setFrame(NSRect(origin: origin, size: size), display: true, animate: panel.isVisible)
        }
        updateGhost()
    }

    func updateGhost() {
        guard let settings = settingsFrame, flow.current?.kind == .drag, [.waitingForDrop, .cancelledPassword].contains(flow.moment), !dragging, !passwordUp else {
            ghost.hide()
            lastGhostRoute = nil
            return
        }
        let start = CGPoint(x: panel.frame.midX, y: panel.frame.maxY - 92)
        let end = CGPoint(x: settings.minX + settings.width * 0.66, y: settings.maxY - 190)
        if let last = lastGhostRoute, hypot(last.0.x - start.x, last.0.y - start.y) < 4, hypot(last.1.x - end.x, last.1.y - end.y) < 4 { return }
        lastGhostRoute = (start, end)
        ghost.show(icon: Self.appIcon, from: start, to: end)
    }

    func granted(_ ids: Set<String>) async -> Set<String> {
        var result = Set<String>()
        let native = await NewPermissions.status()
        for id in ids {
            switch id {
            case "contacts": if CNContactStore.authorizationStatus(for: .contacts) == .authorized { result.insert(id) }
            case "calendar": if EKEventStore.authorizationStatus(for: .event) == .fullAccess { result.insert(id) }
            case "reminders": if EKEventStore.authorizationStatus(for: .reminder) == .fullAccess { result.insert(id) }
            case "full_disk_access", "screen_recording":
                if await Self.probe(id) { result.insert(id) }
            default:
                if ["authorized", "limited"].contains(native[id] ?? "") { result.insert(id) }
            }
        }
        return result
    }

    nonisolated static func probe(_ id: String) async -> Bool {
        await Task.detached {
            guard let executable = Bundle.main.executableURL else { return false }
            let process = Process()
            process.executableURL = executable
            process.arguments = ["permission-probe", id]
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = FileHandle.nullDevice
            guard (try? process.run()) != nil else { return false }
            process.waitUntilExit()
            return String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) == "granted"
        }.value
    }

    nonisolated static func probeHere(_ id: String) -> String {
        switch id {
        case "screen_recording": return CGPreflightScreenCaptureAccess() ? "granted" : "denied"
        case "full_disk_access":
            let file = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/com.apple.TCC/TCC.db")
            guard let handle = try? FileHandle(forReadingFrom: file) else { return "denied" }
            try? handle.close()
            return "granted"
        default: return "denied"
        }
    }

    func save() {
        if flow.finished {
            try? FileManager.default.removeItem(at: Self.resumeFile)
        } else if let data = try? JSONEncoder().encode(flow) {
            try? FileManager.default.createDirectory(at: PrivateReport.directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try? data.write(to: Self.resumeFile, options: .atomic)
        }
    }

    var finishing = false

    func finish() {
        guard !finishing else { return }
        finishing = true
        ghost.hide()
        save()
        if !flow.report.isEmpty { try? PrivateReport.write(flow.summary, to: URL(fileURLWithPath: flow.report)) }
        place()
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2.6))
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.35
                panel.animator().alphaValue = 0
            }, completionHandler: nil)
            try? await Task.sleep(for: .milliseconds(400))
            NSApplication.shared.terminate(nil)
        }
    }
}

struct GuideView: View {
    @ObservedObject var controller: GuideController
    var flow: GuideFlow { controller.flow }

    var body: some View {
        PanelCard(calm: flow.finished) {
            VStack(spacing: 0) {
                if flow.steps.count > 1 && !flow.finished { dots.padding(.top, 18) } else { Spacer().frame(height: 26) }
                hero.frame(height: 112).padding(.top, 10)
                if let step = flow.current {
                    stepText(step).id(flow.index).transition(.asymmetric(insertion: .opacity.combined(with: .offset(x: 18)), removal: .opacity))
                    Spacer(minLength: 10)
                    Button("Not Now") { controller.notNow() }.buttonStyle(PillButtonStyle()).opacity(flow.moment == .allowed ? 0 : 1).padding(.bottom, 26)
                } else {
                    Text("You're all set").font(.system(size: 24, weight: .bold)).padding(.top, 10)
                    Text(flow.allowed.isEmpty ? "You can allow access any time with bridge permissions guide." : "Apple Bridge can now use what you allowed.").font(.system(size: 13, weight: .medium)).foregroundStyle(.secondary).multilineTextAlignment(.center).padding(.top, 8)
                    Spacer(minLength: 10)
                }
            }
            .padding(.horizontal, 22)
            .frame(width: 300, height: 392)
            .animation(.easeInOut(duration: 0.3), value: flow.index)
        }
    }

    func stepText(_ step: GuideStep) -> some View {
        VStack(spacing: 0) {
                    Text(flow.position).font(.system(size: 11, weight: .semibold)).tracking(1.4).foregroundStyle(.secondary).padding(.top, 6)
                    Text("Allow \(step.title)").font(.system(size: 22, weight: .bold)).multilineTextAlignment(.center).padding(.top, 4)
                    Text(instruction(step)).font(.system(size: 14, weight: .medium)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).padding(.top, 8).contentTransition(.opacity)
                    Text(step.reason).font(.system(size: 12)).foregroundStyle(.secondary).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).padding(.top, 6)
        }
        .animation(.easeInOut(duration: 0.2), value: flow.moment)
    }

    var dots: some View {
        HStack(spacing: 6) {
            ForEach(Array(flow.steps.enumerated()), id: \.offset) { index, _ in
                Capsule().fill(index < flow.index ? Palette.allowed : index == flow.index ? Palette.sky : Color.primary.opacity(0.15))
                    .frame(width: index == flow.index ? 18 : 6, height: 6)
            }
        }
        .animation(.spring(response: 0.35), value: flow.index)
    }

    @ViewBuilder var hero: some View {
        if flow.finished || flow.moment == .allowed {
            AllowedMark()
        } else if let step = flow.current, step.kind == .drag {
            VStack(spacing: 6) {
                DragSource(controller: controller).frame(width: 76, height: 76)
                Text(flow.moment == .confirming ? "Waiting for macOS" : "Drag Apple Bridge").font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
            }
        } else if let step = flow.current {
            SymbolTile(symbol: step.symbol)
        }
    }

    func instruction(_ step: GuideStep) -> String {
        switch flow.moment {
        case .waitingForDrop: return "Drag Apple Bridge into the list. If it's already there, switch it on."
        case .confirming: return "Confirm with your password or Touch ID."
        case .cancelledPassword: return "No problem. Drag it in again when you're ready."
        case .allowed: return controller.sheetUp ? "macOS asks to quit Apple Bridge. Click Later. It picks this up by itself." : "\(step.title) is on."
        case .declined: return "Skipped. You can turn it on later in System Settings."
        case .asking: return "Click Allow in the box above."
        case .finished: return ""
        }
    }
}

struct DragSource: NSViewRepresentable {
    let controller: GuideController
    func makeNSView(context: Context) -> DragSourceView {
        let view = DragSourceView()
        view.controller = controller
        return view
    }
    func updateNSView(_ view: DragSourceView, context: Context) {}
}

final class DragSourceView: NSView, NSDraggingSource {
    weak var controller: GuideController?
    let icon = GuideController.appIcon
    var startedDrag = false

    override var intrinsicContentSize: NSSize { NSSize(width: 76, height: 76) }
    override var mouseDownCanMoveWindow: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .openHand) }

    override func draw(_ dirtyRect: NSRect) {
        icon.draw(in: bounds.insetBy(dx: 2, dy: 2))
    }

    override func mouseDown(with event: NSEvent) {
        startedDrag = false
        NSCursor.closedHand.set()
    }

    override func mouseDragged(with event: NSEvent) {
        guard !startedDrag else { return }
        startedDrag = true
        let item = NSDraggingItem(pasteboardWriter: Bundle.main.bundleURL as NSURL)
        item.setDraggingFrame(bounds, contents: icon)
        controller?.dragging = true
        controller?.updateGhost()
        beginDraggingSession(with: [item], event: event, source: self)
    }

    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        context == .outsideApplication ? [.copy, .link, .generic] : []
    }

    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        controller?.dragging = false
        if !operation.isEmpty { controller?.dropped() }
        controller?.updateGhost()
    }
}

@MainActor
final class GhostOverlay {
    let window: NSWindow
    let sprite = CALayer()

    init() {
        let frame = NSScreen.screens.reduce(NSRect.zero) { $0.union($1.frame) }
        window = NSWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.isOpaque = false
        window.backgroundColor = .clear
        window.ignoresMouseEvents = true
        window.level = NSWindow.Level(rawValue: NSWindow.Level.floating.rawValue + 1)
        window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        window.isReleasedWhenClosed = false
        window.hasShadow = false
        let host = NSView(frame: NSRect(origin: .zero, size: frame.size))
        host.wantsLayer = true
        window.contentView = host
        sprite.bounds = CGRect(x: 0, y: 0, width: 56, height: 56)
        sprite.opacity = 0
        let hand = CALayer()
        hand.contents = NSCursor.closedHand.image
        hand.frame = CGRect(x: 30, y: -6, width: 26, height: 26)
        sprite.addSublayer(hand)
        host.layer?.addSublayer(sprite)
    }

    func show(icon: NSImage, from start: CGPoint, to end: CGPoint) {
        let origin = window.frame.origin
        let a = CGPoint(x: start.x - origin.x, y: start.y - origin.y)
        let b = CGPoint(x: end.x - origin.x, y: end.y - origin.y)
        sprite.contents = icon
        sprite.shadowOpacity = 0.3
        sprite.shadowRadius = 8
        sprite.shadowOffset = CGSize(width: 0, height: -3)
        sprite.removeAllAnimations()
        let path = CGMutablePath()
        path.move(to: a)
        path.addQuadCurve(to: b, control: CGPoint(x: (a.x + b.x) / 2, y: max(a.y, b.y) + 40))
        let move = CAKeyframeAnimation(keyPath: "position")
        move.path = path
        move.keyTimes = [0, 1]
        move.beginTime = 0.35
        move.duration = 1.5
        move.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        move.fillMode = .both
        let fade = CAKeyframeAnimation(keyPath: "opacity")
        fade.values = [0, 0.85, 0.85, 0.85, 0]
        fade.keyTimes = [0, 0.12, 0.6, 0.78, 0.92]
        fade.duration = 2.9
        let scale = CAKeyframeAnimation(keyPath: "transform.scale")
        scale.values = [0.9, 1.05, 1.0, 0.9]
        scale.keyTimes = [0, 0.15, 0.62, 0.8]
        scale.duration = 2.9
        let group = CAAnimationGroup()
        group.animations = [move, fade, scale]
        group.duration = 2.9
        group.repeatCount = .infinity
        sprite.position = a
        sprite.add(group, forKey: "ghost")
        window.orderFrontRegardless()
    }

    func hide() {
        sprite.removeAllAnimations()
        sprite.opacity = 0
        window.orderOut(nil)
    }
}
