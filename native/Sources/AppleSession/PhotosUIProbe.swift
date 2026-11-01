#if LAB
import Foundation
import AppKit
import ApplicationServices
import CryptoKit
import SourceSupport

@MainActor
final class PhotosUIProbe {
    private let app: NSRunningApplication
    private let root: AXUIElement
    private let filename: String
    private let assetID: String
    private let markers: [String]
    private var nodes: [AXUIElement] = []
    private var parents: [Int] = []
    private let generation = UUID().uuidString
    private let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev")

    init(assetID: String, filename: String, created: Date) throws {
        guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Photos").first else { throw SourceFailure.unavailable }
        self.app = app
        self.assetID = assetID
        self.filename = filename
        markers = [DateFormatter.Style.medium, .long].map { DateFormatter.localizedString(from: created, dateStyle: $0, timeStyle: .none) }
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
        [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute, kAXHelpAttribute].compactMap { key in
            let value = attribute(node, key)
            return (value as? String) ?? (value as? NSAttributedString)?.string
        }
    }

    private func digest(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func write(_ value: JSON, to url: URL) throws {
        try JSONEncoder().encode(value).write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    private func snapshot(sequence: Int, passed: Bool) throws {
        nodes = []; parents = []
        func collect(_ node: AXUIElement, parent: Int, depth: Int) {
            guard nodes.count < 600, depth < 30 else { return }
            let index = nodes.count
            nodes.append(node); parents.append(parent)
            for child in attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? [] { collect(child, parent: index, depth: depth + 1) }
        }
        for window in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] { collect(window, parent: -1, depth: 0) }
        let known = ["Photos", "Library", "Albums", "Search", "Info", "Delete", "Delete All", "Recover", "Recover All", "Recently Deleted", "View Album", "Unlock", "Add a Title", "Add a Caption", "Add a Keyword", "Cancel", "Close"]
        var groups: [String: [String: JSON]] = [:]
        for (index, node) in nodes.enumerated() {
            let role = attribute(node, kAXRoleAttribute) as? String ?? "unknown"
            let texts = values(node)
            let identifier = attribute(node, kAXIdentifierAttribute) as? String ?? ""
            var fields: [String: JSON] = ["parent_count": .number(Double(parents[index])), "value_count": .number(Double(texts.count)), "values_digest": .string(digest(texts.joined(separator: "\n"))), "identifier_digest": .string(digest(identifier)), "identifier_matched_passed": .bool(!identifier.isEmpty && (identifier.contains(assetID) || identifier.contains(String(assetID.split(separator: "/")[0])))), "filename_matched_passed": .bool(texts.contains { $0.contains(filename) }), "date_matched_passed": .bool(texts.contains { text in markers.contains { text.contains($0) } }), "dimensions_matched_passed": .bool(texts.contains { $0.range(of: "(?<![0-9])2\\s*[×xX]\\s*2(?![0-9])", options: .regularExpression) != nil }), "selected_passed": .bool((attribute(node, kAXSelectedAttribute) as? Bool) == true)]
            for name in known where texts.contains(where: { $0.caseInsensitiveCompare(name) == .orderedSame || $0.hasPrefix(name + " (") }) { fields[name] = .bool(true) }
            var actions: CFArray?
            _ = AXUIElementCopyActionNames(node, &actions)
            fields["press_supported_passed"] = .bool((actions as? [String] ?? []).contains(kAXPressAction))
            groups[role, default: [:]][String(index)] = .object(fields)
        }
        let report: JSON = .object(["generation_digest": .string(digest(generation)), "sequence_count": .number(Double(sequence)), "action_passed": .bool(passed), "node_count": .number(Double(nodes.count)), "nodes": .object(groups.mapValues(JSON.object))])
        try write(report, to: directory.appendingPathComponent("photos-real-ui-state.json"))
    }

    private func key(_ code: CGKeyCode, flags: CGEventFlags = []) async throws {
        guard app.activate() else { throw SourceFailure.unavailable }
        try await Task.sleep(for: .milliseconds(100))
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier,
              let source = CGEventSource(stateID: .privateState),
              let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
    }

    private func search() async throws {
        try await key(3, flags: .maskCommand)
        try await Task.sleep(for: .milliseconds(300))
        guard let focused = attribute(root, kAXFocusedUIElementAttribute), CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw SourceFailure.unavailable }
        let field = unsafeDowncast(focused, to: AXUIElement.self)
        guard (attribute(field, kAXSubroleAttribute) as? String) == "AXSearchField" || values(field).contains(where: { $0.localizedCaseInsensitiveContains("search") }) else { throw SourceFailure.conflict }
        try await key(0, flags: .maskCommand)
        let query = Array((filename as NSString).deletingPathExtension.utf16)
        guard let source = CGEventSource(stateID: .privateState), let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true), let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else { throw SourceFailure.unavailable }
        query.withUnsafeBufferPointer { down.keyboardSetUnicodeString(stringLength: query.count, unicodeString: $0.baseAddress!) }
        down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
        try await key(36)
    }

    private func press(_ index: Int) async throws {
        guard nodes.indices.contains(index) else { throw SourceFailure.invalid }
        let node = nodes[index]
        let role = attribute(node, kAXRoleAttribute) as? String ?? ""
        var ancestor = index
        for _ in 0..<30 {
            guard ancestor >= 0 else { break }
            if values(nodes[ancestor]).contains(where: { text in ["Delete", "Recover", "Unlock", "View Album", "Share", "Import", "Use Touch ID", "Edit"].contains { text.hasPrefix($0) } }) { throw SourceFailure.unsupported }
            ancestor = parents[ancestor]
        }
        if role == kAXRowRole {
            guard AXUIElementSetAttributeValue(node, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else { throw SourceFailure.unavailable }
        } else {
            guard role == kAXImageRole || (role == kAXButtonRole && values(node).contains(where: { $0.hasPrefix("Search") || $0 == "Info" })) else { throw SourceFailure.unsupported }
            if AXUIElementPerformAction(node, kAXPressAction as CFString) != .success {
                guard role == kAXImageRole, let position = attribute(node, kAXPositionAttribute), let size = attribute(node, kAXSizeAttribute),
                      CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw SourceFailure.unavailable }
                var point = CGPoint.zero; var extent = CGSize.zero
                guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &point), AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &extent), extent.width > 0, extent.height > 0, app.activate() else { throw SourceFailure.unavailable }
                try await Task.sleep(for: .milliseconds(100))
                let center = CGPoint(x: point.x + extent.width / 2, y: point.y + extent.height / 2)
                guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier,
                      let source = CGEventSource(stateID: .privateState), let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: center, mouseButton: .left), let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: center, mouseButton: .left) else { throw SourceFailure.unavailable }
                down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
            }
        }
        app.activate()
        try await Task.sleep(for: .milliseconds(300))
    }

    func run() async throws {
        var sequence = 0
        try await Task.sleep(for: .milliseconds(300))
        try snapshot(sequence: sequence, passed: true)
        let deadline = Date().addingTimeInterval(300)
        let commandURL = directory.appendingPathComponent("photos-real-ui-action.json")
        while Date() < deadline {
            try await Task.sleep(for: .milliseconds(200))
            guard let data = try? Data(contentsOf: commandURL), let action = try? JSONDecoder().decode(JSON.self, from: data), action["generation_digest"].string == digest(generation), action["sequence_count"].number > Double(sequence) else { continue }
            sequence = Int(action["sequence_count"].number)
            if action["operation"].string == "finish" { return }
            var passed = true
            do {
                switch action["operation"].string {
                case "snapshot": break
                case "search": try await search()
                case "press": try await press(Int(action["node_count"].number))
                case "info": try await key(34, flags: .maskCommand)
                case "return": try await key(36)
                case "escape": try await key(53)
                case "tab": try await key(48)
                case "right": try await key(124)
                case "left": try await key(123)
                case "down": try await key(125)
                case "up": try await key(126)
                default: throw SourceFailure.unsupported
                }
            } catch { passed = false }
            try await Task.sleep(for: .milliseconds(400))
            try snapshot(sequence: sequence, passed: passed)
        }
    }
}
#endif
