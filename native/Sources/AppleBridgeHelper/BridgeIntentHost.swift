import AppKit
import AppIntents
import CoreServices
import ApplicationServices

@MainActor
enum BridgeIntentHost {
    static func run(_ arguments: [String]) {
        guard arguments.count == 3, let parent = Int32(arguments[1]), parent > 1 else { return }
        let report = URL(fileURLWithPath: arguments[2])
        LSRegisterURL(Bundle.main.bundleURL as CFURL, true)
        BridgeShortcuts.updateAppShortcutParameters()
        let watcher = Task { @MainActor in
            let deadline = Date().addingTimeInterval(300)
            while let text = shortcutDialog(), Date() < deadline {
                publish(text)
                try? await Task.sleep(for: .seconds(1))
            }
            try? Data(String(getpid()).utf8).write(to: report, options: .atomic)
            var previous = ""
            while !Task.isCancelled {
                if let text = shortcutDialog(), text != previous { publish(text); previous = text }
                try? await Task.sleep(for: .seconds(1))
            }
        }
        defer { watcher.cancel() }
        let lifetime = DispatchSource.makeProcessSource(identifier: parent, eventMask: .exit, queue: .main)
        lifetime.setEventHandler { NSApplication.shared.terminate(nil) }
        lifetime.resume()
        defer { lifetime.cancel() }
        if kill(parent, 0) == 0 { NSApplication.shared.run() }
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
        guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.shortcuts").first else { return nil }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.2)
        func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
            var value: CFTypeRef?
            AXUIElementCopyAttributeValue(element, key as CFString, &value)
            return value
        }
        for window in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] {
            var texts: [String] = [], buttons: [String] = [], budget = 1000
            func visit(_ element: AXUIElement, _ depth: Int) {
                guard budget > 0, depth < 16 else { return }
                budget -= 1
                let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                if role == kAXStaticTextRole || role == kAXButtonRole {
                    let text = (attribute(element, kAXValueAttribute) as? String).flatMap { $0.isEmpty ? nil : $0 } ?? attribute(element, kAXTitleAttribute) as? String ?? ""
                    if !text.isEmpty { texts.append(text); if role == kAXButtonRole { buttons.append(text) } }
                }
                for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { visit(child, depth + 1) }
            }
            visit(window, 0)
            if buttons.contains(where: { ["Always Allow", "Allow", "Don’t Allow", "Don't Allow", "Add Shortcut", "Replace"].contains($0) }) {
                return texts.joined(separator: " | ")
            }
        }
        return nil
    }
}
