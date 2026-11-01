#if LAB
import Foundation
import AppKit
import ApplicationServices
import SourceSupport

@MainActor
public enum FocusPhonePack {
    public static func install(_ file: URL) async throws -> JSON {
        guard AXIsProcessTrusted() else { return .object(["accessibility_passed": .bool(false), "pack_import_passed": .bool(false)]) }
        guard file.lastPathComponent == "AB Focus.shortcut", file.isFileURL, FileManager.default.fileExists(atPath: file.path) else { throw SourceFailure.invalid }
        let previous = NSWorkspace.shared.frontmostApplication
        let wasRunning = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.shortcuts").first != nil
        defer {
            if !wasRunning { NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.shortcuts").first?.terminate() }
            if let previous, previous.bundleIdentifier != "com.apple.shortcuts" { previous.activate(options: []) }
        }
        NSWorkspace.shared.open(file)
        var added = false
        var replaced = false
        let deadline = Date().addingTimeInterval(35)
        while Date() < deadline {
            if let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.shortcuts").first {
                let root = AXUIElementCreateApplication(app.processIdentifier)
                AXUIElementSetMessagingTimeout(root, 2)
                let windows = children(root, kAXWindowsAttribute)
                var pending = false
                for window in windows {
                    var budget = 1500
                    let nodes = descend(window, budget: &budget)
                    let labels = nodes.flatMap { node in [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].compactMap { string(node, $0) } }
                    guard labels.contains(where: { $0 == "AB Focus" || $0.contains("“AB Focus”") || $0.contains("\"AB Focus\"") }) else { continue }
                    let buttons = nodes.filter { string($0, kAXRoleAttribute) == kAXButtonRole }
                    let replace = buttons.filter { ["Replace", "Replace Shortcut"].contains(string($0, kAXTitleAttribute) ?? string($0, kAXDescriptionAttribute) ?? "") }
                    let add = buttons.filter { (string($0, kAXTitleAttribute) ?? string($0, kAXDescriptionAttribute)) == "Add Shortcut" }
                    if added, replace.count == 1 {
                        pending = true
                        replaced = AXUIElementPerformAction(replace[0], kAXPressAction as CFString) == .success
                    } else if !added, add.count == 1 {
                        pending = true
                        added = AXUIElementPerformAction(add[0], kAXPressAction as CFString) == .success
                    } else if !add.isEmpty || !replace.isEmpty { pending = true }
                }
                if added && replaced && !pending { return .object(["accessibility_passed": .bool(true), "pack_import_passed": .bool(true)]) }
            }
            try await Task.sleep(for: .milliseconds(250))
        }
        return .object(["accessibility_passed": .bool(true), "pack_import_passed": .bool(false)])
    }
    private static func children(_ node: AXUIElement, _ attribute: String = kAXChildrenAttribute) -> [AXUIElement] {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, attribute as CFString, &value) == .success else { return [] }
        return value as? [AXUIElement] ?? []
    }
    private static func string(_ node: AXUIElement, _ attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, attribute as CFString, &value) == .success else { return nil }
        return value as? String
    }
    private static func descend(_ node: AXUIElement, budget: inout Int) -> [AXUIElement] {
        guard budget > 0 else { return [] }
        budget -= 1
        var nodes = [node]
        for child in children(node) { nodes += descend(child, budget: &budget) }
        return nodes
    }
}
#endif
