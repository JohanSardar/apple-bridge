#if LAB
import Foundation
import AppKit
import ApplicationServices
@preconcurrency import Photos
import SourceSupport

struct PhotosProofAsset {
    let id: String
    let filename: String
    let created: Date
}

@MainActor
final class PhotosProofUI {
    private let app: NSRunningApplication
    private let root: AXUIElement
    var stage = "opening Photos"
    var diagnostics: [String: JSON] = [:]

    init() throws {
        guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Photos").first else { throw SourceFailure.unavailable }
        self.app = app
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 1)
        _ = AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        _ = AXUIElementSetAttributeValue(root, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    }

    private func attribute(_ node: AXUIElement, _ key: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(node, key as CFString, &value) == .success else { return nil }
        return value
    }

    private func values(_ node: AXUIElement) -> [String] {
        [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].compactMap {
            let value = attribute(node, $0)
            return (value as? String) ?? (value as? NSAttributedString)?.string
        }
    }

    private func elements(_ role: String? = nil, below node: AXUIElement? = nil) -> [AXUIElement] {
        var result: [AXUIElement] = []
        var budget = 2500
        @MainActor func visit(_ item: AXUIElement, _ depth: Int) {
            guard budget > 0, depth < 30 else { return }
            budget -= 1
            let itemRole = attribute(item, kAXRoleAttribute) as? String
            if role == nil || itemRole == role { result.append(item) }
            for child in attribute(item, kAXChildrenAttribute) as? [AXUIElement] ?? [] { visit(child, depth + 1) }
        }
        if let node { visit(node, 0) }
        else { for window in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] { visit(window, 0) } }
        return result
    }

    func inspect() {
        var roles: [String: Int] = [:]
        var controls: [String: JSON] = [:]
        let known = ["Delete", "Delete Photo", "Delete Photos", "Delete 1 Photo", "Delete All", "Cancel", "Search", "View Album", "Unlock", "Info", "Recently Deleted", "Library"]
        for node in elements() {
            roles[attribute(node, kAXRoleAttribute) as? String ?? "unknown", default: 0] += 1
            for text in values(node) where known.contains(text) { controls[text] = .bool(true) }
        }
        diagnostics["roles"] = .object(roles.mapValues { .number(Double($0)) })
        diagnostics["controls"] = .object(controls)
    }

    private func deleteButtons(_ sheet: AXUIElement) -> [AXUIElement] {
        elements(kAXButtonRole, below: sheet).filter { node in values(node).contains { $0.hasPrefix("Delete") && !$0.localizedCaseInsensitiveContains("all") } }
    }

    private func key(_ code: CGKeyCode, flags: CGEventFlags = []) async throws {
        guard app.activate() else { throw SourceFailure.unavailable }
        for _ in 0..<20 {
            if NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier,
              let source = CGEventSource(stateID: .privateState),
              let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
        try await Task.sleep(for: .milliseconds(250))
    }

    private func closeInfo() async throws {
        if elements(kAXWindowRole).contains(where: { values($0).contains("Info") }) {
            try await key(34, flags: .maskCommand)
            try await Task.sleep(for: .milliseconds(300))
        }
    }

    private func chooseSidebar(_ name: String) async throws {
        try await closeInfo()
        let sheets = elements(kAXSheetRole)
        diagnostics["pending_sheet_count"] = .number(Double(sheets.count))
        if sheets.count == 1, deleteButtons(sheets[0]).count == 1 {
            let cancel = elements(kAXButtonRole, below: sheets[0]).filter { values($0).contains("Cancel") }
            if cancel.count == 1, AXUIElementPerformAction(cancel[0], kAXPressAction as CFString) == .success {
                diagnostics["pending_delete_cancelled_passed"] = .bool(true)
                try await Task.sleep(for: .milliseconds(500))
            }
        }
        guard elements(kAXSheetRole).isEmpty else { throw SourceFailure.conflict }
        let rows = elements(kAXRowRole).filter { row in elements(below: row).contains { values($0).contains(name) } }
        diagnostics[name == "Library" ? "library_row_count" : "trash_row_count"] = .number(Double(rows.count))
        guard rows.count == 1, AXUIElementSetAttributeValue(rows[0], kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else { throw SourceFailure.unavailable }
        try await Task.sleep(for: .milliseconds(500))
    }

    private func search(_ filename: String) async throws {
        stage = "searching for the verified test photo"
        try await chooseSidebar("Library")
        try await key(53)
        for _ in 0..<20 {
            let fields = elements(kAXTextFieldRole).filter { (attribute($0, kAXSubroleAttribute) as? String) == "AXSearchField" }
            if !fields.isEmpty { break }
            let buttons = elements(kAXButtonRole).filter { values($0).contains("Search") }
            if let button = buttons.first { _ = AXUIElementPerformAction(button, kAXPressAction as CFString) }
            else { try await key(3, flags: .maskCommand) }
            try await Task.sleep(for: .milliseconds(300))
        }
        let fields = elements(kAXTextFieldRole).filter { (attribute($0, kAXSubroleAttribute) as? String) == "AXSearchField" }
        diagnostics["search_field_count"] = .number(Double(fields.count))
        guard fields.count == 1 else { throw SourceFailure.unavailable }
        _ = AXUIElementSetAttributeValue(fields[0], kAXFocusedAttribute as CFString, kCFBooleanTrue)
        try await key(0, flags: .maskCommand)
        let query = Array((filename as NSString).deletingPathExtension.utf16)
        guard let source = CGEventSource(stateID: .privateState), let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true), let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else { throw SourceFailure.unavailable }
        query.withUnsafeBufferPointer { down.keyboardSetUnicodeString(stringLength: query.count, unicodeString: $0.baseAddress!) }
        down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
        try await Task.sleep(for: .milliseconds(600))
        try await key(36)
        try await Task.sleep(for: .milliseconds(700))
    }

    private func dateMatches(_ node: AXUIElement, _ assets: [PhotosProofAsset]) -> Bool {
        let markers = assets.flatMap { asset in [DateFormatter.Style.medium, .long].map { DateFormatter.localizedString(from: asset.created, dateStyle: $0, timeStyle: .none) } }
        return values(node).contains { text in markers.contains { text.contains($0) } }
    }

    private func candidates(_ assets: [PhotosProofAsset]) -> [AXUIElement] {
        elements(kAXGroupRole).filter { node in
            guard let parent = attribute(node, kAXParentAttribute), CFGetTypeID(parent) == AXUIElementGetTypeID(),
                  attribute(unsafeDowncast(parent, to: AXUIElement.self), kAXRoleAttribute) as? String == kAXListRole else { return false }
            return dateMatches(node, assets)
        }
    }

    private func select(_ node: AXUIElement) async throws {
        guard let position = attribute(node, kAXPositionAttribute), let size = attribute(node, kAXSizeAttribute),
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw SourceFailure.unavailable }
        var origin = CGPoint.zero
        var extent = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin),
              AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &extent),
              extent.width > 0, extent.height > 0, app.activate() else { throw SourceFailure.unavailable }
        try await Task.sleep(for: .milliseconds(200))
        let center = CGPoint(x: origin.x + extent.width / 2, y: origin.y + extent.height / 2)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier,
              let source = CGEventSource(stateID: .privateState),
              let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: center, mouseButton: .left),
              let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: center, mouseButton: .left) else { throw SourceFailure.unavailable }
        down.setIntegerValueField(.mouseEventClickState, value: 1)
        up.setIntegerValueField(.mouseEventClickState, value: 1)
        down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
        diagnostics["selection_click_passed"] = .bool(true)
    }

    private func inspectSelection(_ node: AXUIElement, assets: [PhotosProofAsset]) async throws -> Bool {
        stage = "selecting the verified search result"
        guard dateMatches(node, assets), elements(kAXSheetRole).isEmpty else { throw SourceFailure.conflict }
        try await select(node)
        try await Task.sleep(for: .milliseconds(350))
        stage = "checking the selected test photo in Info"
        try await key(34, flags: .maskCommand)
        for _ in 0..<20 {
            let panels = elements(kAXWindowRole).filter { values($0).contains("Info") }
            diagnostics["info_panel_count"] = .number(Double(panels.count))
            if panels.count == 1 {
                let texts = elements(below: panels[0]).flatMap(values)
                let named = texts.contains { text in assets.contains { text == $0.filename } }
                let tiny = texts.contains { $0.range(of: "(?<![0-9])2\\s*[×xX]\\s*2(?![0-9])", options: .regularExpression) != nil }
                diagnostics["info_value_count"] = .number(Double(texts.count))
                diagnostics["info_filename_passed"] = .bool(named)
                diagnostics["info_dimensions_passed"] = .bool(tiny)
                if named && tiny {
                    diagnostics["selection_filename_passed"] = .bool(true)
                    diagnostics["selection_dimensions_passed"] = .bool(true)
                    diagnostics["selection_date_passed"] = .bool(true)
                    try await closeInfo()
                    return true
                }
                if texts.contains(where: { $0.range(of: "\\.(png|jpe?g|heic|heif|tiff?|gif|mov|mp4)$", options: [.regularExpression, .caseInsensitive]) != nil }) {
                    try await closeInfo()
                    return false
                }
            }
            try await Task.sleep(for: .milliseconds(150))
        }
        try await closeInfo()
        throw SourceFailure.unavailable
    }

    private func confirmDelete(permanent: Bool, deleted: (@MainActor () -> Bool)? = nil) async throws {
        guard elements(kAXSheetRole).isEmpty else { throw SourceFailure.conflict }
        try await key(51)
        for _ in 0..<40 {
            let sheets = elements(kAXSheetRole)
            diagnostics["delete_sheet_count"] = .number(Double(sheets.count))
            guard sheets.count <= 1 else { throw SourceFailure.conflict }
            if let sheet = sheets.first {
                let buttons = deleteButtons(sheet)
                diagnostics["delete_button_count"] = .number(Double(buttons.count))
                guard buttons.count == 1, attribute(buttons[0], kAXEnabledAttribute) as? Bool == true,
                      AXUIElementPerformAction(buttons[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unavailable }
                diagnostics["delete_button_pressed_passed"] = .bool(true)
                for _ in 0..<30 {
                    if elements(kAXSheetRole).isEmpty {
                        diagnostics[permanent ? "permanent_delete_confirmed_passed" : "photos_delete_confirmed_passed"] = .bool(true)
                        return
                    }
                    try await Task.sleep(for: .milliseconds(150))
                }
                throw SourceFailure.unavailable
            }
            if sheets.isEmpty, let deleted, deleted() {
                diagnostics["photos_delete_observed_passed"] = .bool(true)
                return
            }
            try await Task.sleep(for: .milliseconds(150))
        }
        throw SourceFailure.unavailable
    }

    private func live(_ assets: [PhotosProofAsset]) -> Set<String> {
        let fetched = PHAsset.fetchAssets(withLocalIdentifiers: assets.map(\.id), options: nil)
        var ids: Set<String> = []
        fetched.enumerateObjects { asset, _, _ in ids.insert(asset.localIdentifier) }
        return ids
    }

    func trash(_ assets: [PhotosProofAsset]) async throws {
        for filename in Set(assets.map(\.filename)).sorted() {
            let group = assets.filter { $0.filename == filename }
            for _ in 0..<group.count {
                let before = live(group)
                if before.isEmpty { break }
                try await search(filename)
                var matches: [AXUIElement] = []
                for _ in 0..<40 {
                    matches = candidates(group)
                    if matches.count == before.count { break }
                    try await Task.sleep(for: .milliseconds(250))
                }
                diagnostics["search_result_count"] = .number(Double(matches.count))
                guard matches.count == before.count, let selected = matches.first,
                      try await inspectSelection(selected, assets: group) else { throw SourceFailure.conflict }
                stage = "deleting the verified test photo in Photos"
                try await confirmDelete(permanent: false, deleted: { self.live(group).count == before.count - 1 })
                var after = before
                for _ in 0..<60 {
                    after = live(group)
                    if after.count == before.count - 1 { break }
                    try await Task.sleep(for: .milliseconds(150))
                }
                guard after.isSubset(of: before), before.subtracting(after).count == 1 else { throw SourceFailure.conflict }
            }
            guard live(group).isEmpty else { throw SourceFailure.unavailable }
        }
        diagnostics["active_test_asset_count"] = .number(0)
    }

    func openTrash() async throws {
        stage = "opening Recently Deleted"
        try await chooseSidebar("Recently Deleted")
        for _ in 0..<30 {
            let buttons = elements(kAXButtonRole)
            if buttons.contains(where: { values($0).contains("View Album") || values($0).contains("Unlock") }) {
                diagnostics["recently_deleted_locked_passed"] = .bool(true)
                throw SourceFailure.permissionMissing
            }
            if buttons.contains(where: { values($0).contains { $0.hasPrefix("Recover") } }) {
                diagnostics["recently_deleted_unlocked_passed"] = .bool(true)
                return
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        throw SourceFailure.unavailable
    }

    private func permanentlyRemoved(_ assets: [PhotosProofAsset]) throws -> Bool {
        let database = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Pictures/Photos Library.photoslibrary/database/Photos.sqlite")
        guard FileManager.default.fileExists(atPath: database.path) else { throw SourceFailure.unavailable }
        func quoted(_ value: String) -> String { "'" + value.replacingOccurrences(of: "'", with: "''") + "'" }
        let predicates = try assets.map { asset in
            if let uuid = asset.id.split(separator: "/").first, UUID(uuidString: String(uuid)) != nil {
                return "a.ZUUID=" + quoted(String(uuid))
            }
            guard asset.id.isEmpty, asset.filename.hasPrefix("[ab-test] "), asset.created.timeIntervalSinceReferenceDate.isFinite else { throw SourceFailure.invalid }
            return "b.ZORIGINALFILENAME=" + quoted(asset.filename) + " AND ABS(a.ZDATECREATED-\(asset.created.timeIntervalSinceReferenceDate))<1"
        }
        guard !predicates.isEmpty else { throw SourceFailure.invalid }
        let task = Process()
        let output = Pipe()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        task.arguments = ["-readonly", database.path, "SELECT COUNT(*) FROM ZASSET a LEFT JOIN ZADDITIONALASSETATTRIBUTES b ON a.ZADDITIONALATTRIBUTES=b.Z_PK WHERE " + predicates.map { "(" + $0 + ")" }.joined(separator: " OR ") + ";"]
        task.standardOutput = output
        task.standardError = FileHandle.nullDevice
        try task.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        task.waitUntilExit()
        guard task.terminationStatus == 0, let text = String(data: data, encoding: .utf8), let remaining = Int(text.trimmingCharacters(in: .whitespacesAndNewlines)) else { throw SourceFailure.unavailable }
        diagnostics["remaining_owned_database_asset_count"] = .number(Double(remaining))
        return remaining == 0
    }

    private func verifyPermanentCleanup(_ assets: [PhotosProofAsset]) async throws {
        for _ in 0..<40 {
            if try permanentlyRemoved(assets) {
                diagnostics["recently_deleted_cleanup_passed"] = .bool(true)
                return
            }
            try await Task.sleep(for: .milliseconds(250))
        }
        throw SourceFailure.conflict
    }

    func purge(_ assets: [PhotosProofAsset]) async throws {
        try await openTrash()
        try await Task.sleep(for: .seconds(1))
        for _ in 0..<assets.count {
            var found = false
            for node in candidates(assets) {
                if try await inspectSelection(node, assets: assets) {
                    stage = "permanently deleting the verified test photo in Photos"
                    try await confirmDelete(permanent: true)
                    found = true
                    break
                }
            }
            if !found { try await verifyPermanentCleanup(assets); return }
        }
        for node in candidates(assets) {
            if try await inspectSelection(node, assets: assets) { throw SourceFailure.conflict }
        }
        try await verifyPermanentCleanup(assets)
    }

    func remove(_ id: String, filename: String, created: Date) async throws {
        let assets = [PhotosProofAsset(id: id, filename: filename, created: created)]
        try await trash(assets)
        try await purge(assets)
    }
}
#endif
