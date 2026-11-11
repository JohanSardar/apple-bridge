import AppKit
import SwiftUI

@MainActor
final class SetupGuide {
    let panel: NSPanel
    private var following: Task<Void, Never>?

    init(model: SetupModel) {
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 392, height: 150), styleMask: [.titled, .fullSizeContentView, .nonactivatingPanel, .utilityWindow], backing: .buffered, defer: false)
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.isReleasedWhenClosed = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.standardWindowButton(.closeButton)?.isHidden = true
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true
        panel.contentView = FirstMouseHostingView(rootView: GuideView(model: model))
    }

    func show() {
        panel.setFrame(target(), display: true)
        panel.orderFrontRegardless()
        following?.cancel()
        following = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                self?.follow()
                try? await Task.sleep(for: .milliseconds(400))
            }
        }
    }

    func follow() {
        guard panel.isVisible else { return }
        let frame = target()
        guard abs(frame.minX - panel.frame.minX) > 1 || abs(frame.minY - panel.frame.minY) > 1 else { return }
        panel.setFrame(frame, display: true, animate: true)
    }

    func close() {
        following?.cancel()
        following = nil
        panel.orderOut(nil)
    }

    private func target() -> NSRect {
        let size = panel.frame.size
        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let height = NSScreen.screens.first?.frame.height ?? screen.maxY
        let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        let settings = windows.first { ($0[kCGWindowOwnerName as String] as? String) == "System Settings" && ($0[kCGWindowLayer as String] as? Int) == 0 }
        guard let bounds = settings?[kCGWindowBounds as String] as? NSDictionary, let rect = CGRect(dictionaryRepresentation: bounds) else {
            return NSRect(x: screen.maxX - size.width - 24, y: screen.maxY - size.height - 24, width: size.width, height: size.height)
        }
        let window = NSRect(x: rect.minX, y: height - rect.maxY, width: rect.width, height: rect.height)
        let top = min(window.maxY - 56, screen.maxY - 12) - size.height
        if window.maxX + 16 + size.width <= screen.maxX { return NSRect(x: window.maxX + 16, y: top, width: size.width, height: size.height) }
        if window.minX - 16 - size.width >= screen.minX { return NSRect(x: window.minX - 16 - size.width, y: top, width: size.width, height: size.height) }
        return NSRect(x: window.maxX - size.width - 24, y: window.minY + 24, width: size.width, height: size.height)
    }
}

struct GuideView: View {
    @ObservedObject var model: SetupModel
    @State private var nudge = false
    var body: some View {
        let permission = model.current
        let done = permission.map { model.celebrating == $0.id } ?? false
        HStack(spacing: 14) {
            DraggableApp(size: 52)
                .offset(x: nudge && !done ? -6 : 0)
                .animation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true), value: nudge)
            VStack(alignment: .leading, spacing: 5) {
                if done {
                    Label("\(permission?.title ?? "Permission") is on", systemImage: "checkmark.circle.fill")
                        .font(.headline).foregroundStyle(.green)
                    Text("Taking you back to setup…").font(.callout).foregroundStyle(.secondary)
                } else {
                    Text("Drag Apple Bridge into the \(permission?.title ?? "") list").font(.headline).fixedSize(horizontal: false, vertical: true)
                    Text("Then turn on its switch. If it's already listed, just turn it on. If macOS offers to quit Apple Bridge, click Later.").font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.mini)
                        Text("Waiting…").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Back to Setup") { model.leaveSettings() }.controlSize(.small)
                    }
                    .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .frame(width: 392, height: 150)
        .background(.regularMaterial)
        .onAppear { nudge = true }
    }
}

final class FirstMouseHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}
