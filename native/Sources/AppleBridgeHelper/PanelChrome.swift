import AppKit
import SwiftUI

final class FloatingPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    init(size: NSSize) {
        super.init(contentRect: NSRect(origin: .zero, size: size), styleMask: [.nonactivatingPanel, .borderless, .fullSizeContentView], backing: .buffered, defer: false)
        isFloatingPanel = true
        becomesKeyOnlyIfNeeded = true
        hidesOnDeactivate = false
        level = .floating
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        isMovableByWindowBackground = false
        isReleasedWhenClosed = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        animationBehavior = .utilityWindow
    }
}

enum Palette {
    static let sky = Color(red: 0.18, green: 0.48, blue: 1.0)
    static let lagoon = Color(red: 0.07, green: 0.74, blue: 0.72)
    static let mint = Color(red: 0.55, green: 0.93, blue: 0.62)
    static let ink = Color(nsColor: .labelColor)
    static let allowed = Color(red: 0.13, green: 0.72, blue: 0.42)
}

struct Aurora: View {
    var calm = false
    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / 30)) { timeline in
            let t = timeline.date.timeIntervalSinceReferenceDate * (calm ? 0.35 : 0.8)
            Canvas { context, size in
                context.addFilter(.blur(radius: 26))
                let blobs: [(Color, Double, Double)] = [(Palette.sky, 0.0, 0.22), (Palette.lagoon, 2.1, 0.5), (Palette.mint, 4.2, 0.78)]
                for (color, phase, anchor) in blobs {
                    let x = size.width * (anchor + 0.16 * sin(t + phase))
                    let y = size.height * (0.82 + 0.08 * cos(t * 1.3 + phase))
                    let radius = size.width * (0.30 + 0.05 * sin(t * 0.9 + phase))
                    context.fill(Path(ellipseIn: CGRect(x: x - radius, y: y - radius * 0.55, width: radius * 2, height: radius * 1.1)), with: .color(color.opacity(0.7)))
                }
            }
        }
        .allowsHitTesting(false)
    }
}

struct PanelCard<Content: View>: View {
    var calm = false
    @ViewBuilder var content: Content
    var body: some View {
        content
            .frame(maxWidth: .infinity)
            .background {
                ZStack {
                    Rectangle().fill(.thickMaterial)
                    Rectangle().fill(Color(nsColor: .windowBackgroundColor).opacity(0.55))
                    Aurora(calm: calm).mask(LinearGradient(colors: [.clear, .black], startPoint: .center, endPoint: .bottom))
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Color.white.opacity(0.35), lineWidth: 1))
    }
}

struct PillButtonStyle: ButtonStyle {
    var prominent = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .semibold))
            .padding(.horizontal, 18).padding(.vertical, 8)
            .frame(minWidth: 96)
            .foregroundStyle(prominent ? Color.white : Palette.ink)
            .background(Capsule().fill(prominent ? AnyShapeStyle(LinearGradient(colors: [Palette.sky, Palette.lagoon], startPoint: .leading, endPoint: .trailing)) : AnyShapeStyle(Color.primary.opacity(0.08))))
            .overlay(Capsule().strokeBorder(Color.primary.opacity(prominent ? 0 : 0.12)))
            .opacity(configuration.isPressed ? 0.75 : 1)
            .contentShape(Capsule())
    }
}

struct AlertButtonStyle: ButtonStyle {
    var prominent = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: prominent ? .semibold : .regular))
            .frame(maxWidth: .infinity, minHeight: 28)
            .foregroundStyle(prominent ? Color.white : Color(nsColor: .labelColor))
            .background(Capsule().fill(prominent ? Color(nsColor: .controlAccentColor) : Color.primary.opacity(0.1)))
            .brightness(configuration.isPressed ? -0.08 : 0)
            .contentShape(Capsule())
    }
}

struct AlertBackground: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 22, style: .continuous)
            .fill(.regularMaterial)
            .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Color.primary.opacity(0.12), lineWidth: 0.5))
    }
}

struct AllowedMark: View {
    @State private var shown = false
    var body: some View {
        ZStack {
            Circle().fill(Palette.allowed.opacity(0.16)).frame(width: 78, height: 78).scaleEffect(shown ? 1 : 0.6)
            Image(systemName: "checkmark.circle.fill").font(.system(size: 54, weight: .semibold)).foregroundStyle(Palette.allowed).scaleEffect(shown ? 1 : 0.4)
        }
        .onAppear { withAnimation(.spring(response: 0.38, dampingFraction: 0.6)) { shown = true } }
    }
}

struct SymbolTile: View {
    let symbol: String
    var body: some View {
        RoundedRectangle(cornerRadius: 18, style: .continuous)
            .fill(LinearGradient(colors: [Palette.sky, Palette.lagoon], startPoint: .topLeading, endPoint: .bottomTrailing))
            .frame(width: 72, height: 72)
            .overlay(Image(systemName: symbol).font(.system(size: 32, weight: .semibold)).foregroundStyle(.white))
            .shadow(color: Palette.sky.opacity(0.35), radius: 10, y: 4)
    }
}

enum ScreenGeometry {
    struct Window { let owner: String; let frame: CGRect }

    static func windows() -> [Window] {
        guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return [] }
        let primary = NSScreen.screens.first?.frame.height ?? 0
        return list.compactMap { window in
            guard let owner = window[kCGWindowOwnerName as String] as? String,
                  let bounds = window[kCGWindowBounds as String] as? NSDictionary,
                  let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary), rect.width > 120, rect.height > 80 else { return nil }
            return Window(owner: owner, frame: CGRect(x: rect.minX, y: primary - rect.maxY, width: rect.width, height: rect.height))
        }
    }

    static var settingsWindow: CGRect? {
        windows().filter { $0.owner == "System Settings" }.map(\.frame).max { $0.width * $0.height < $1.width * $1.height }
    }

    static var dialogShowing: Bool {
        let all = windows()
        let settings = all.filter { $0.owner == "System Settings" }
        let system = all.contains { window in
            let name = window.owner.lowercased()
            return name.contains("securityagent") || name.contains("coreautha") || name.contains("localauthentication") || name.contains("usernotificationcenter")
        }
        return system || settings.count > 1
    }
}
