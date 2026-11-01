#if LAB
import Foundation
import AppKit
import ApplicationServices
import SourceSupport

@MainActor
public enum NotesOwnedCleanup {
    private struct Failure: Error { let stage: String }
    private static func value(_ node: AXUIElement, _ key: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, key as CFString, &result) == .success else { return "" }
        return result as? String ?? ""
    }
    private static func children(_ node: AXUIElement) -> [AXUIElement] {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, kAXChildrenAttribute as CFString, &result) == .success else { return [] }
        return result as? [AXUIElement] ?? []
    }
    private static func nodes(_ root: AXUIElement, budget: inout Int) -> [AXUIElement] {
        guard budget > 0 else { return [] }
        budget -= 1
        return [root] + children(root).flatMap { nodes($0, budget: &budget) }
    }
    private static func all(_ root: AXUIElement) -> [AXUIElement] {
        var budget = 12000
        return nodes(root, budget: &budget)
    }
    private static func enabled(_ node: AXUIElement) -> Bool {
        var result: CFTypeRef?
        return AXUIElementCopyAttributeValue(node, kAXEnabledAttribute as CFString, &result) == .success && result as? Bool == true
    }
    private static func text(_ node: AXUIElement, contains tag: String) -> Bool {
        [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains { value(node, $0).contains(tag) }
    }
    private static func confirm(_ root: AXUIElement, tag: String) throws -> Bool {
        let dialogs = all(root).filter { node in
            let role = value(node, kAXRoleAttribute), subrole = value(node, kAXSubroleAttribute)
            return (role == kAXSheetRole || subrole == "AXDialog") && all(node).contains { text($0, contains: tag) }
        }
        if dialogs.isEmpty { return false }
        guard dialogs.count == 1 else { throw Failure(stage: "ambiguous_owned_dialog") }
        let buttons = all(dialogs[0]).filter { value($0, kAXRoleAttribute) == kAXButtonRole && ["Delete", "Delete Folder"].contains(value($0, kAXTitleAttribute)) && enabled($0) }
        guard buttons.count == 1, AXUIElementPerformAction(buttons[0], kAXPressAction as CFString) == .success else { throw Failure(stage: "owned_delete_confirmation") }
        return true
    }
    public static func run(_ request: JSON) async -> JSON {
        guard AXIsProcessTrusted() else { return .object(["passed": .bool(false), "stage": .string("accessibility_permission_missing")]) }
        let tag = request["tag"].string, folders = request["folders"].array
        guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\n"), !folders.isEmpty else { return .object(["passed": .bool(false), "stage": .string("invalid_owned_request")]) }
        let store = NotesStore(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path)
        var removed = 0
        do {
            let snapshot = try store.read(keys: []).observations
            var targets: [(String, String, String)] = []
            for owned in folders {
                let id = owned["id"].string, title = owned["title"].string
                guard !id.isEmpty, title == "Bridge Test " + tag || title.hasPrefix("Bridge Test " + tag + " ") else { throw Failure(stage: "unowned_folder") }
                guard let folder = snapshot.first(where: { $0["collection"].string == "notes.folder" && $0["scoped_alias"].string == id }) else { continue }
                let key = folder["native_key"].string
                guard folder["data"]["title"].string == title, !folder["data"]["read_only"].bool,
                      !snapshot.contains(where: { $0["data"]["folder_id"]["$ref"]["native_key"].string == key || $0["data"]["parent_id"]["$ref"]["native_key"].string == key }) else { throw Failure(stage: "owned_folder_not_empty") }
                targets.append((id, title, key))
            }
            for (id, title, key) in targets {
                guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Notes").first else { throw Failure(stage: "notes_not_running") }
                let root = AXUIElementCreateApplication(app.processIdentifier)
                app.activate()
                if try !confirm(root, tag: tag) {
                    _ = try NotesScript.run("show folder id " + NotesScript.quote(id))
                    try await Task.sleep(for: .milliseconds(300))
                    let rows = all(root).filter { node in
                        value(node, kAXRoleAttribute) == kAXRowRole && all(node).contains { child in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains { value(child, $0) == title } }
                    }
                    guard rows.count == 1 else { throw Failure(stage: "owned_sidebar_row_" + String(rows.count)) }
                    guard AXUIElementPerformAction(rows[0], kAXShowMenuAction as CFString) == .success else { throw Failure(stage: "owned_folder_context_menu") }
                    try await Task.sleep(for: .milliseconds(200))
                    let items = all(root).filter { node in
                        value(node, kAXRoleAttribute) == kAXMenuItemRole && ["Delete Folder", "Delete Folder…", "Delete Folder...", "Delete"].contains(value(node, kAXTitleAttribute)) && enabled(node)
                    }
                    guard items.count == 1, AXUIElementPerformAction(items[0], kAXPressAction as CFString) == .success else { throw Failure(stage: "owned_delete_menu_" + String(items.count)) }
                    try await Task.sleep(for: .milliseconds(300))
                    _ = try confirm(root, tag: tag)
                }
                var verified = false
                for _ in 0..<200 {
                    if try ["gone", "marked", "trashed"].contains(store.deletionState(key, collection: "notes.folder")) { verified = true; break }
                    try await Task.sleep(for: .milliseconds(50))
                }
                guard verified else { throw Failure(stage: "owned_folder_notestore_unverified") }
                removed += 1
            }
            return .object(["passed": .bool(true), "removed_count": .number(Double(removed))])
        } catch {
            return .object(["passed": .bool(false), "removed_count": .number(Double(removed)), "stage": .string((error as? Failure)?.stage ?? "owned_cleanup_unavailable")])
        }
    }
}
#endif
