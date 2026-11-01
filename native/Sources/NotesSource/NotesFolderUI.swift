import Foundation
import AppKit
import ApplicationServices
import SourceSupport

@MainActor
public enum NotesFolderUI {
    private struct Failure: Error { let stage: String }
    private static var deadline = Date.distantPast
    private static func value(_ node: AXUIElement, _ key: String) -> String {
        guard Date() < deadline else { return "" }
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, key as CFString, &result) == .success else { return "" }
        return result as? String ?? ""
    }
    private static func children(_ node: AXUIElement) -> [AXUIElement] {
        guard Date() < deadline else { return [] }
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, kAXChildrenAttribute as CFString, &result) == .success else { return [] }
        return result as? [AXUIElement] ?? []
    }
    private static func nodes(_ root: AXUIElement, budget: inout Int) -> [AXUIElement] {
        guard Date() < deadline, budget > 0, value(root, kAXRoleAttribute) != kAXMenuBarRole else { return [] }
        budget -= 1
        return [root] + children(root).flatMap { nodes($0, budget: &budget) }
    }
    private static func all(_ root: AXUIElement) -> [AXUIElement] {
        var budget = 12000
        return nodes(root, budget: &budget)
    }
    private static func enabled(_ node: AXUIElement) -> Bool {
        guard Date() < deadline else { return false }
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
    public static var available: Bool { AXIsProcessTrusted() }
    public static func rename(id: String, from oldTitle: String, to newTitle: String, trace: CaptureTrace? = nil) async throws {
        var stage = "permission"
        defer { trace?.record("notes_folder_rename_ui", fields: ["source": .string("notes"), "stage": .string(stage)]) }
        guard available else { throw SourceFailure.permissionMissing }
        guard oldTitle.hasPrefix("Bridge Test [ab-test "), newTitle.hasPrefix("Bridge Test [ab-test ") else { throw SourceFailure.unsupported }
        deadline = Date().addingTimeInterval(8)
        guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Notes").first else { throw SourceFailure.unavailable }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.5)
        _ = app.activate()
        stage = "show_folder"
        _ = try NotesScript.run("show folder id " + NotesScript.quote(id))
        try await Task.sleep(for: .milliseconds(250))
        stage = "find_row"
        let rows = all(root).filter { node in value(node, kAXRoleAttribute) == kAXRowRole && all(node).contains { child in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains { [oldTitle, newTitle].contains(value(child, $0)) } } }
        guard rows.count == 1, AXUIElementPerformAction(rows[0], kAXShowMenuAction as CFString) == .success else { throw SourceFailure.unavailable }
        try await Task.sleep(for: .milliseconds(200))
        stage = "find_rename_menu"
        let menus = all(root).filter { value($0, kAXRoleAttribute) == kAXMenuItemRole && ["Rename Folder", "Rename Folder…", "Rename Folder...", "Rename"].contains(value($0, kAXTitleAttribute)) && enabled($0) }
        guard menus.count == 1, AXUIElementPerformAction(menus[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unavailable }
        try await Task.sleep(for: .milliseconds(200))
        stage = "find_name_field"
        let fields = all(root).filter { value($0, kAXRoleAttribute) == kAXTextFieldRole && [oldTitle, newTitle].contains(value($0, kAXValueAttribute)) }
        guard fields.count == 1, AXUIElementSetAttributeValue(fields[0], kAXValueAttribute as CFString, newTitle as CFString) == .success else { throw SourceFailure.unavailable }
        stage = "commit_name"
        if AXUIElementPerformAction(fields[0], kAXConfirmAction as CFString) != .success {
            var focused: CFTypeRef?
            guard Date() < deadline, NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier,
                  AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success,
                  let focused, CFEqual(focused, fields[0]), value(fields[0], kAXValueAttribute) == newTitle,
                  let down = CGEvent(keyboardEventSource: nil, virtualKey: 36, keyDown: true),
                  let up = CGEvent(keyboardEventSource: nil, virtualKey: 36, keyDown: false) else { throw SourceFailure.conflict }
            down.post(tap: .cghidEventTap)
            up.post(tap: .cghidEventTap)
        }
    }
    #if LAB
    public static func run(_ request: JSON) async -> JSON {
        deadline = Date().addingTimeInterval(30)
        guard AXIsProcessTrusted() else { return .object(["passed": .bool(false), "stage": .string("accessibility_permission_missing")]) }
        let tag = request["tag"].string, folders = request["folders"].array
        guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\n"), !folders.isEmpty else { return .object(["passed": .bool(false), "stage": .string("invalid_owned_request")]) }
        let store = NotesStore(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path)
        var removed = 0
        var stage = "validate_owned_folders"
        do {
            let snapshot = try store.read(metadataOnly: true).observations
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
                if try ["gone", "marked", "trashed"].contains(store.deletionState(key, collection: "notes.folder")) { removed += 1; continue }
                stage = "revalidate_owned_folder"
                let fresh = try store.read(metadataOnly: true).observations
                guard fresh.contains(where: { $0["scoped_alias"].string == id && $0["data"]["title"].string == title }),
                      !fresh.contains(where: { $0["data"]["folder_id"]["$ref"]["native_key"].string == key || $0["data"]["parent_id"]["$ref"]["native_key"].string == key }) else { throw Failure(stage: "owned_folder_changed") }
                guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Notes").first else { throw Failure(stage: "notes_not_running") }
                let root = AXUIElementCreateApplication(app.processIdentifier)
                AXUIElementSetMessagingTimeout(root, 0.5)
                _ = app.activate()
                stage = "confirm_owned_folder"
                if try !confirm(root, tag: title) {
                    stage = "show_owned_folder"
                    _ = try NotesScript.run("show folder id " + NotesScript.quote(id))
                    try await Task.sleep(for: .milliseconds(300))
                    stage = "find_owned_sidebar_row"
                    let rows = all(root).filter { node in
                        value(node, kAXRoleAttribute) == kAXRowRole && all(node).contains { child in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains { value(child, $0) == title } }
                    }
                    guard rows.count == 1 else { throw Failure(stage: "owned_sidebar_row_" + String(rows.count)) }
                    stage = "open_owned_context_menu"
                    guard AXUIElementPerformAction(rows[0], kAXShowMenuAction as CFString) == .success else { throw Failure(stage: "owned_folder_context_menu") }
                    try await Task.sleep(for: .milliseconds(200))
                    stage = "find_owned_delete_menu"
                    let items = all(root).filter { node in
                        value(node, kAXRoleAttribute) == kAXMenuItemRole && ["Delete Folder", "Delete Folder…", "Delete Folder...", "Delete"].contains(value(node, kAXTitleAttribute)) && enabled(node)
                    }
                    guard items.count == 1, AXUIElementPerformAction(items[0], kAXPressAction as CFString) == .success else { throw Failure(stage: "owned_delete_menu_" + String(items.count)) }
                    try await Task.sleep(for: .milliseconds(300))
                    _ = try confirm(root, tag: title)
                }
                stage = "verify_owned_folder_deletion"
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
            return .object(["passed": .bool(false), "removed_count": .number(Double(removed)), "stage": .string((error as? Failure)?.stage ?? stage + "_" + ((error as? SourceFailure).map { String(describing: $0) } ?? "unavailable"))])
        }
    }
    #endif
}
