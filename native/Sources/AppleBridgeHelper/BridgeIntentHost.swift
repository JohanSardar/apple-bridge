import AppKit
import AppIntents
import SwiftUI
import CoreServices
import ApplicationServices

@MainActor
enum BridgeIntentHost {
    static func runApplication(_ controller: PackagedApp? = nil) {
        BridgeIntentApplicationDelegate.controller = controller
        BridgeIntentApplication.main()
    }

    static func run(_ arguments: [String]) {
        guard arguments.count == 3, let parent = Int32(arguments[1]), parent > 1 else { return }
        let report = URL(fileURLWithPath: arguments[2])
        LSRegisterURL(Bundle.main.bundleURL as CFURL, true)
        BridgeShortcuts.updateAppShortcutParameters()
        try? Data(String(getpid()).utf8).write(to: report, options: .atomic)
        let watcher = Task { @MainActor in
            let deadline = Date().addingTimeInterval(300)
            var previous = ""
            while let text = shortcutDialog(), Date() < deadline {
                if text != previous { publish(text); previous = text }
                try? await Task.sleep(for: .seconds(1))
            }
            try? Data().write(to: report.appendingPathExtension("ready"), options: .atomic)
            while !Task.isCancelled {
                if let text = shortcutDialog() {
                    if text != previous { publish(text); previous = text }
                } else { previous = "" }
                try? await Task.sleep(for: .seconds(1))
            }
        }
        defer { watcher.cancel() }
        let lifetime = DispatchSource.makeProcessSource(identifier: parent, eventMask: .exit, queue: .main)
        lifetime.setEventHandler { NSApplication.shared.terminate(nil) }
        lifetime.resume()
        defer { lifetime.cancel() }
        if kill(parent, 0) == 0 { runApplication() }
    }

    private static func publish(_ text: String) {
        let file = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("F9-DIALOG.txt")
        let line = text.replacingOccurrences(of: "\n", with: " ") + "\n"
        if let handle = try? FileHandle(forWritingTo: file) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: Data(line.utf8))
        } else { try? Data(line.utf8).write(to: file, options: .atomic) }
    }

    private static func shortcutDialog() -> String? {
        guard AXIsProcessTrusted() else { return nil }
        let applications = NSWorkspace.shared.runningApplications.filter {
            let identifier = $0.bundleIdentifier ?? ""
            return identifier.hasPrefix("com.apple.shortcuts") || identifier.hasPrefix("com.apple.WorkflowKit") || identifier == "com.apple.clock"
        }
        for app in applications {
            let root = AXUIElementCreateApplication(app.processIdentifier)
            AXUIElementSetMessagingTimeout(root, 0.2)
            func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
                var value: CFTypeRef?
                AXUIElementCopyAttributeValue(element, key as CFString, &value)
                return value
            }
            for window in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] {
                var texts: [String] = [], buttons: [String] = [], budget = 1000
                var hasDialog = false
                func visit(_ element: AXUIElement, _ depth: Int) {
                    guard budget > 0, depth < 16 else { return }
                    budget -= 1
                    let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                    if role == kAXSheetRole || attribute(element, kAXSubroleAttribute) as? String == kAXDialogSubrole { hasDialog = true }
                    if role == kAXStaticTextRole || role == kAXButtonRole {
                        var text = ""
                        for key in [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute] {
                            if let value = attribute(element, key) as? String, !value.isEmpty { text = value; break }
                        }
                        if !text.isEmpty { texts.append(text); if role == kAXButtonRole { buttons.append(text) } }
                    }
                    for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { visit(child, depth + 1) }
                }
                visit(window, 0)
                if app.bundleIdentifier == "com.apple.clock" && !buttons.contains(where: { $0 == "Done" || $0.hasPrefix("Delete") }) { continue }
                if !texts.isEmpty && !buttons.isEmpty && (hasDialog || buttons.contains(where: { ["Always Allow", "Allow", "Don’t Allow", "Don't Allow", "Add Shortcut", "Replace", "Continue", "OK"].contains($0) })) {
                    return texts.joined(separator: " | ")
                }
            }
        }
        return nil
    }
}

private struct BridgeIntentApplication: App {
    @NSApplicationDelegateAdaptor(BridgeIntentApplicationDelegate.self) private var delegate

    var body: some Scene {
        Settings { EmptyView() }
    }
}

@MainActor
private final class BridgeIntentApplicationDelegate: NSObject, NSApplicationDelegate {
    static var controller: PackagedApp?

    func applicationDidFinishLaunching(_ notification: Notification) {
        Self.controller?.show()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        Self.controller?.applicationShouldHandleReopen(sender, hasVisibleWindows: flag) ?? false
    }
}
