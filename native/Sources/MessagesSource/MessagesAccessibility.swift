import Foundation
import Decoders
import AppKit
import ApplicationServices
import SourceSupport

public enum MessagesAction: String, Sendable {
    case send, reply, react, edit, unsend, markRead = "mark_read", markUnread = "mark_unread", deleteMessage = "delete_message", deleteChat = "delete_chat", createGroup = "create_group", renameGroup = "rename_group", addMember = "add_member", removeMember = "remove_member", leaveGroup = "leave_group"
    public init(operation: String) throws {
        guard let value = Self(rawValue: String(operation.dropFirst("messages.".count))) else { throw SourceFailure.invalid }
        self = value
    }
}

@UIActor
enum MessagesAccessibility {
    static var available: Bool { AXIsProcessTrusted() }
    static func value(_ element: AXUIElement, _ attribute: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? (result as? URL)?.absoluteString ?? result as? String ?? ""
    }
    static func children(_ element: AXUIElement) -> [AXUIElement] {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &result) == .success else { return [] }
        return result as? [AXUIElement] ?? []
    }
    static func matching(_ root: AXUIElement, budget: inout Int, predicate: (AXUIElement) -> Bool) -> [AXUIElement] {
        guard budget > 0, value(root, kAXRoleAttribute) != kAXMenuBarRole else { return [] }
        budget -= 1
        var result = predicate(root) ? [root] : []
        for child in children(root) { result += matching(child, budget: &budget, predicate: predicate) }
        return result
    }
    static func uiSession() throws -> MessagesUISession {
        do { return try MessagesUISession.frontmost(trusted: available) } catch {
            try? PrivateReport.write(["stage": "frontmost_application"], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-prepare-diagnostic.json").path))
            throw error
        }
    }
    static func mediaPreviewNames(_ bubble: AXUIElement, names: Set<String>) throws -> [JSON] {
        var actions: CFArray?
        guard AXUIElementCopyActionNames(bubble, &actions) == .success else { return [] }
        let preview = (actions as? [String] ?? []).filter { $0.hasPrefix("Name:Quick Look\n") }
        guard preview.count == 1 else { return [] }
        let session = try uiSession()
        func previewWindows() -> [AXUIElement] {
            let services = NSWorkspace.shared.runningApplications.filter { $0.bundleIdentifier == "com.apple.quicklook.QuickLookUIService" }.map { AXUIElementCreateApplication($0.processIdentifier) }
            return ([session.root] + services).flatMap { root in
                var windows: CFTypeRef?
                AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &windows)
                return windows as? [AXUIElement] ?? children(root).filter { value($0, kAXRoleAttribute) == kAXWindowRole }
            }
        }
        let previous = previewWindows()
        try session.fence()
        guard AXUIElementPerformAction(bubble, preview[0] as CFString) == .success else { return [] }
        defer {
            try? session.press(MessagesUISession.escape)
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        let deadline = Date().addingTimeInterval(1)
        var found: Set<String> = []
        var windowCount = 0
        var stemMatches = 0
        let stems = Set(names.map { URL(fileURLWithPath: $0).deletingPathExtension().lastPathComponent })
        repeat {
            let windows = previewWindows().filter { window in !previous.contains(where: { CFEqual($0, window) }) }
            windowCount = max(windowCount, windows.count)
            var budget = 1000
            for window in windows {
                _ = matching(window, budget: &budget) { node in
                    var attributes: CFArray?
                    AXUIElementCopyAttributeNames(node, &attributes)
                    for attribute in attributes as? [String] ?? [] {
                        let text = value(node, attribute)
                        if stems.contains(text) { stemMatches += 1 }
                        if MessagesMediaCleanupProof.matches(text, names: names) { found.insert(text) }
                    }
                    return false
                }
            }
            if !found.isEmpty { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        let digest = BodyDigest.hash(Data(value(bubble, kAXDescriptionAttribute).utf8))
        try? PrivateReport.write(["new_window_count": windowCount, "filename_matches": found.count, "stem_matches": stemMatches, "messages_frontmost": NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.MobileSMS"], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-media-preview-" + digest + ".json").path))
        return found.map(JSON.string)
    }
    static func target(_ root: AXUIElement, prepared: Prepared) throws -> AXUIElement {
        try MessagesUIActions.resolve(until: Date().addingTimeInterval(3), wait: {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }, attempt: { try targetNow(root, prepared: prepared) })
    }
    private static func targetNow(_ root: AXUIElement, prepared: Prepared) throws -> AXUIElement {
        let guid = prepared.action["native_key"].string
        guard !guid.isEmpty else { throw SourceFailure.capabilityUnproven }
        var budget = 4000
        let expected = prepared.nativeState["target"]["text"].string
        let mediaNames = prepared.nativeState["cleanup_authorized"].bool ? prepared.nativeState["target"]["cleanup_media_names"].array.map(\.string) : []
        guard !expected.isEmpty || !mediaNames.isEmpty else { throw SourceFailure.conflict }
        let outgoing = prepared.nativeState["target"]["is_from_me"].bool
        if prepared.nativeState["conversation_target"].bool {
            let rows = matching(root, budget: &budget) { element in
                if MessagesUIActions.conversationEntry(role: value(element, kAXRoleAttribute), description: value(element, kAXDescriptionAttribute), expected: expected) { return true }
                guard value(element, kAXRoleAttribute) == kAXRowRole else { return false }
                var textBudget = 100
                return matching(element, budget: &textBudget) { node in [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains { [expected, "You: " + expected].contains(value(node, $0)) } }.count > 0 && textBudget > 0
            }
            guard budget > 0, rows.count == 1 else {
                var diagnosticBudget = 4000
                let candidates = matching(root, budget: &diagnosticBudget) { value($0, kAXRoleAttribute) == kAXRowRole }
                let samples = candidates.map { row -> [String: Any] in
                    var nested = 100
                    let exact = matching(row, budget: &nested) { node in
                        [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains { value(node, $0).contains(expected) }
                    }.count
                    return ["contains_expected_count": exact, "budget_remaining": nested]
                }
                try? PrivateReport.write(["stage": "conversation_target", "matching_rows": rows.count, "budget_remaining": budget, "rows": samples], to: PrivateReport.directory.appendingPathComponent("messages-target-diagnostic.json"))
                throw MessagesUIFailure.targetUnavailable
            }
            return rows[0]
        }
        if !mediaNames.isEmpty {
            let bubbles = matching(root, budget: &budget) { element in
                let identifier = value(element, kAXIdentifierAttribute)
                return identifier == "Sticker" || identifier == guid || identifier == "message-" + guid
            }
            var candidates: [JSON] = []
            for bubble in bubbles {
                var nested = 150
                var values: [JSON] = []
                _ = matching(bubble, budget: &nested) { child in
                    values += [kAXDescriptionAttribute, kAXValueAttribute, "AXURL", kAXTitleAttribute, "AXFilename", kAXHelpAttribute, "AXDocument", "AXValueDescription"].map { .string(value(child, $0)) }
                    return false
                }
                if [guid, "message-" + guid].contains(value(bubble, kAXIdentifierAttribute)) { values += mediaNames.map(JSON.string) }
                let description = value(bubble, kAXDescriptionAttribute)
                let direction = MessagesMediaCleanupProof.direction(description, names: Set(mediaNames)).map(JSON.bool) ?? .null
                candidates.append(.object(["outgoing": direction, "values": nested > 0 ? .array(values) : .array([])]))
            }
            let ownedCopies = max(1, Int(prepared.nativeState["target"]["cleanup_media_owned_copies"].number))
            let selfEcho = prepared.nativeState["target"]["cleanup_self_echo"].bool
            var selected = MessagesMediaCleanupProof.targetIndex(candidates, names: Set(mediaNames), outgoing: outgoing, allowSelfEcho: selfEcho, equivalentOwned: prepared.nativeState["target"]["cleanup_equivalent_owned_media"].bool, ownedCopies: ownedCopies)
            if selected == nil, budget > 0 {
                let deadline = Date().addingTimeInterval(6)
                for (index, bubble) in bubbles.enumerated() {
                    guard Date() < deadline else { throw MessagesUIFailure.targetUnavailable }
                    let names = try mediaPreviewNames(bubble, names: Set(mediaNames))
                    if !names.isEmpty {
                        var candidate = candidates[index].object
                        candidate["values"] = .array(candidates[index]["values"].array + names)
                        candidates[index] = .object(candidate)
                        selected = MessagesMediaCleanupProof.targetIndex(candidates, names: Set(mediaNames), outgoing: outgoing, allowSelfEcho: selfEcho, ownedCopies: ownedCopies)
                        if selected != nil { break }
                    }
                }
                selected = MessagesMediaCleanupProof.targetIndex(candidates, names: Set(mediaNames), outgoing: outgoing, allowSelfEcho: selfEcho, equivalentOwned: prepared.nativeState["target"]["cleanup_equivalent_owned_media"].bool, ownedCopies: ownedCopies)
            }
            guard budget > 0, let index = selected else {
                var diagnosticBudget = 4000
                var nodes: [[String: Any]] = []
                _ = matching(root, budget: &diagnosticBudget) { node in
                    var attributes: CFArray?
                    AXUIElementCopyAttributeNames(node, &attributes)
                    let names = attributes as? [String] ?? []
                    let guidMatches = names.filter { value(node, $0).contains(guid) }
                    let filenameMatches = names.filter { attribute in mediaNames.contains(where: value(node, attribute).contains) }
                    let identifier = value(node, kAXIdentifierAttribute)
                    if identifier == "Sticker" || !guidMatches.isEmpty || !filenameMatches.isEmpty {
                        var selected: CFTypeRef?
                        AXUIElementCopyAttributeValue(node, kAXSelectedAttribute as CFString, &selected)
                        nodes.append(["media_bubble": identifier == "Sticker", "guid_attribute_matches": guidMatches.count, "filename_attribute_matches": filenameMatches.count, "attribute_count": names.count, "selected": (selected as? NSNumber)?.boolValue ?? false, "outgoing_description": value(node, kAXDescriptionAttribute).hasPrefix("Your iMessage,"), "incoming_description": value(node, kAXDescriptionAttribute).hasPrefix("iMessage,"), "description_length": value(node, kAXDescriptionAttribute).count])
                    }
                    return false
                }
                try? PrivateReport.write(["budget_remaining": budget, "nodes": nodes, "candidate_value_matches": candidates.map { $0["values"].array.filter { MessagesMediaCleanupProof.matches($0.string, names: Set(mediaNames)) }.count }], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-media-target-diagnostic.json").path))
                throw MessagesUIFailure.targetUnavailable
            }
            return bubbles[index]
        }
        let targets = matching(root, budget: &budget) { element in
            let identifier = value(element, kAXIdentifierAttribute)
            let description = value(element, kAXDescriptionAttribute)
            var textBudget = 100
            let exactText = identifier == "Sticker" && matching(element, budget: &textBudget) { value($0, kAXIdentifierAttribute) == "CKBalloonTextView" && textValue($0) == expected }.count == 1
            return exactText && (prepared.nativeState["cleanup_authorized"].bool || MessagesUIActions.matchesDirection(description, outgoing: outgoing))
        }
        guard !targets.isEmpty, budget > 0 else {
            var diagnosticBudget = 1000
            let bubbles = matching(root, budget: &diagnosticBudget) { value($0, kAXIdentifierAttribute) == "Sticker" }
            let samples = bubbles.map { bubble -> [String: Any] in
                var nestedBudget = 50
                let texts = matching(bubble, budget: &nestedBudget) { value($0, kAXIdentifierAttribute) == "CKBalloonTextView" }.map { textValue($0) }
                return ["outgoing": value(bubble, kAXDescriptionAttribute).hasPrefix("Your iMessage,"), "description_contains_expected": value(bubble, kAXDescriptionAttribute).contains(expected), "text_lengths": texts.map(\.count), "text_matches": texts.map { $0 == expected }]
            }
            let report: [String: Any] = ["expected_length": expected.count, "expected_outgoing": outgoing, "budget_remaining": budget, "bubbles": samples]
            try? PrivateReport.write(report, to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-target-diagnostic.json").path))
            throw MessagesUIFailure.targetUnavailable
        }
        guard targets.count == 1 || prepared.nativeState["cleanup_authorized"].bool else { throw MessagesUIFailure.targetAmbiguous }
        let directed = targets.filter { MessagesUIActions.matchesDirection(value($0, kAXDescriptionAttribute), outgoing: outgoing) }
        return directed.first ?? targets[0]
    }
    private static func focus(_ root: AXUIElement) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &result) == .success, let result else { return "" }
        guard CFGetTypeID(result)==AXUIElementGetTypeID() else {return ""}
        let element = unsafeDowncast(result, to: AXUIElement.self)
        return value(element, kAXIdentifierAttribute) + "|" + value(element, kAXRoleAttribute)
    }
    static func validate(_ prepared: Prepared) throws {
        let root = try uiSession().root
        let deadline = Date().addingTimeInterval(4)
        var selected: AXUIElement?
        repeat {
            selected = try? target(root, prepared: prepared)
            if selected != nil { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        guard let bubble = selected else { throw MessagesUIFailure.targetUnavailable }
        if prepared.action["op"].string == "messages.react", prepared.action["messages"]["reaction"].string == "emoji" {
            var actions: CFArray?
            guard AXUIElementCopyActionNames(bubble, &actions) == .success,
                  (actions as? [String] ?? []).contains(where: { $0.hasPrefix("Name:" + prepared.action["messages"]["emoji"].string + "\n") }) else { throw SourceFailure.capabilityUnproven }
        }
    }
    static func validateConversation(_ prepared: Prepared) throws {
        let chat = prepared.action["native_key"].string
        let message = prepared.nativeState["message_guid"].string
        let reader = try MessagesSelfTestBaseline.reader()
        let memberships = try reader.query("SELECT c.guid FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)", [message]).map { $0["guid"] }
        let deadline = Date().addingTimeInterval(4)
        repeat {
            var budget = 4000
            let texts = matching(try uiSession().root, budget: &budget) { value($0, kAXIdentifierAttribute) == "CKBalloonTextView" }.map { textValue($0) }
            if budget > 0, MessagesSelfTestIsolation.matchesConversation(chat: chat, memberships: memberships, expected: prepared.nativeState["target"]["text"].string, visibleTexts: texts) { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesUIFailure.targetUnavailable
    }
    private static func performReadState(_ prepared: Prepared) throws -> JSON {
        try openMessage(prepared.nativeState["message_guid"].string, settle: 0.5, resetEditor: false)
        try validateConversation(prepared)
        let session = try uiSession()
        let menus = MessagesVariantUI.find(session.root, role: kAXMenuBarItemRole, name: "Conversation")
        guard menus.count == 1 else { throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(menus[0])
        var menuOpen = true
        defer { if menuOpen { try? session.press(MessagesUISession.escape) } }
        let action: String = try MessagesUIActions.resolve(until: Date().addingTimeInterval(3), wait: {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }, attempt: {
            let items = MessagesVariantUI.find(menus[0], role: kAXMenuItemRole)
            let titles = items.map { item in
                let title = value(item, kAXTitleAttribute)
                return title.isEmpty ? value(item, kAXDescriptionAttribute) : title
            }
            guard let action = MessagesUIActions.readStateAction(unread: prepared.action["op"].string == "messages.mark_unread", titles: titles) else { throw MessagesUIFailure.targetUnavailable }
            return action
        })
        if !action.isEmpty {
            let items = MessagesVariantUI.find(menus[0], role: kAXMenuItemRole, name: action)
            guard items.count == 1 else { throw MessagesUIFailure.menuUnavailable }
            try session.fence()
            try MessagesVariantUI.press(items[0])
            menuOpen = false
        }
        return .object(["target_verified": .bool(true)])
    }
    static func press(_ session: MessagesUISession, title: String) throws {
        let deadline = Date().addingTimeInterval(2)
        repeat {
            let matches = MessagesUIActions.buttons(in: session.root, title: title, children: { children($0) }, value: { value($0, $1) })
            if matches.count == 1 {
                try session.fence()
                guard AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        throw MessagesUIFailure.menuUnavailable
    }
    private static func edit(_ session: MessagesUISession, text: String, previous: String) throws {
        let deadline = Date().addingTimeInterval(3)
        repeat {
            var budget = 2000
            let fields = matching(session.root, budget: &budget) { value($0, kAXRoleAttribute) == kAXTextFieldRole && value($0, kAXIdentifierAttribute) != "messageBodyField" && textValue($0) == previous }
            if fields.count == 1 {
                try session.type(text, into: fields[0], value: textValue)
                try session.press(MessagesUISession.enter)
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        throw MessagesUIFailure.editFieldUnavailable
    }
    private static func center(_ element: AXUIElement) throws -> CGPoint {
        var position: CFTypeRef?
        var size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size, CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw SourceFailure.conflict }
        var origin = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin), AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions), dimensions.width > 0, dimensions.height > 0 else { throw SourceFailure.conflict }
        return CGPoint(x: origin.x + dimensions.width / 2, y: origin.y + dimensions.height / 2)
    }
    static func contextMenu(_ element: AXUIElement, session: MessagesUISession) throws {
        _ = AXUIElementPerformAction(element, "AXScrollToVisible" as CFString)
        RunLoop.current.run(until: Date().addingTimeInterval(0.15))
        try session.controlClick(at: center(element))
    }
    static func replace(_ session: MessagesUISession, text: String) throws {
        let root = session.root
        let deadline = Date().addingTimeInterval(2)
        repeat {
            var focused: CFTypeRef?
            var editor: AXUIElement?
            if AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() {
                let field = unsafeDowncast(focused, to: AXUIElement.self)
                if value(field, kAXIdentifierAttribute) == "messageBodyField" { editor = field }
            }
            if editor == nil {
                var budget = 4000
                let fields = matching(root, budget: &budget) { value($0, kAXIdentifierAttribute) == "messageBodyField" }
                if fields.count == 1 { editor = fields[0] }
            }
            if let editor {
                let previous = textValue(editor)
                guard MessagesUISession.canReplaceDraft(previous, with: text) else { throw MessagesUIFailure.draftPresent }
                try session.type(text, into: editor, value: textValue)
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        throw MessagesUIFailure.composerUnavailable
    }
    private static func snapshot(_ root: AXUIElement, prepared: Prepared) -> JSON {
        var budget = 4000
        let nodes = matching(root, budget: &budget) { value($0, kAXIdentifierAttribute) == "TranscriptCollectionView" }
        return .object(["transcript_present": .bool(nodes.count == 1), "target_text": .string(BodyDigest.hash(Data(prepared.nativeState["target"]["text"].string.utf8)))])
    }
    static func acknowledgeDeletedInfo() throws -> Bool {
        let session = try uiSession(), root = session.root
        let windows = children(root).filter { value($0, kAXRoleAttribute) == kAXWindowRole }
        let sheets = windows.flatMap { children($0) }.filter { value($0, kAXRoleAttribute) == kAXSheetRole }
        guard !sheets.isEmpty else { return false }
        var budget = 200
        let notices = sheets.flatMap { sheet in matching(sheet, budget: &budget) { value($0, kAXRoleAttribute) == kAXStaticTextRole && value($0, kAXValueAttribute).hasPrefix("Deleted messages are moved to Recently") } }
        guard notices.count == 1 else { return false }
        try press(session, title: "OK")
        RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        return true
    }
    private static func resetTaggedEditor(_ tag: String) throws -> Bool {
        let session = try uiSession(), root = session.root
        var budget = 2000
        let cancel = matching(root, budget: &budget) { value($0, kAXDescriptionAttribute) == "Cancel edit" }
        guard cancel.count == 1 else { return false }
        guard tag.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]$"#, options: .regularExpression) != nil else { throw MessagesUIFailure.draftPresent }
        budget = 2000
        let fields = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXTextFieldRole && value($0, kAXIdentifierAttribute) != "messageBodyField" && textValue($0).hasPrefix(tag) }
        guard !fields.isEmpty else { throw MessagesUIFailure.draftPresent }
        guard AXUIElementPerformAction(cancel[0], kAXPressAction as CFString) == .success else {
            try? PrivateReport.write(["stage": "cancel_tagged_editor"], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-prepare-diagnostic.json").path))
            throw SourceFailure.conflict
        }
        try session.press(MessagesUISession.escape)
        RunLoop.current.run(until: Date().addingTimeInterval(0.35))
        return true
    }
    static func openMessage(_ guid: String, settle: TimeInterval = 3, resetEditor: Bool = true, ownedTag: String = "") throws {
        let url = try MessagesNavigation.messageURL(guid)
        MessagesUISession.activate()
        if resetEditor {
            _ = try resetTaggedEditor(ownedTag)
        } else {
            let root = try uiSession().root
            var budget = 2000
            guard matching(root, budget: &budget, predicate: { value($0, kAXDescriptionAttribute) == "Cancel edit" }).isEmpty, budget > 0 else { throw SourceFailure.conflict }
        }
        try MessagesNavigation.open(url)
        RunLoop.current.run(until: Date().addingTimeInterval(settle))
    }
    private static func textValue(_ element: AXUIElement) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? ""
    }
    static func inspectRetractedTarget(_ guid: String) throws {
        try openMessage(guid, settle: 0.5, resetEditor: false)
        var budget = 4000
        let nodes = matching(try uiSession().root, budget: &budget) { _ in true }
        let labels = Set(["You unsent a message", "You unsent a message.", "This message was unsent", "Delete", "Delete…"])
        let values = nodes.map { element -> [String: Any] in
            let identifier = value(element, kAXIdentifierAttribute)
            var actions: CFArray?
            AXUIElementCopyActionNames(element, &actions)
            return ["role": value(element, kAXRoleAttribute), "identifier": identifier.range(of: #"^[A-Za-z_]+$"#, options: .regularExpression) != nil ? identifier : "", "guid_matches": identifier == guid || identifier == "message-" + guid, "retracted_label": [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].contains { labels.contains(value(element, $0)) }, "delete_action": (actions as? [String] ?? []).contains { $0.hasPrefix("Name:Delete") }]
        }
        try PrivateReport.write(["nodes": values, "budget_remaining": budget], to: PrivateReport.directory.appendingPathComponent("messages-retracted-target-diagnostic.json"))
    }
    static func clearTaggedDraft(_ tag: String, groupHandles: [String] = []) throws -> Bool {
        MessagesUISession.activate()
        if !groupHandles.isEmpty {
            var components = URLComponents()
            components.scheme = "imessage"
            components.path = "open"
            components.queryItems = [URLQueryItem(name: "addresses", value: groupHandles.joined(separator: ","))]
            guard let url = components.url else { throw MessagesUIFailure.navigationUnavailable }
            try MessagesNavigation.open(url)
            RunLoop.current.run(until: Date().addingTimeInterval(3))
        }
        let session = try uiSession()
        var budget = 4000
        let fields = matching(session.root, budget: &budget) { value($0, kAXIdentifierAttribute) == "messageBodyField" }
        guard fields.count == 1 else { return false }
        let field = fields[0]
        let text = textValue(field)
        guard text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil,
              tag.isEmpty || text.contains(tag) else { return false }
        return try session.clear(field, value: textValue)
    }
    static func createGroup(_ args: JSON) throws {
        let handles = args["handles"].array.map(\.string)
        var components = URLComponents()
        components.scheme = "imessage"
        components.path = "open"
        components.queryItems = [URLQueryItem(name: "addresses", value: handles.joined(separator: ","))]
        guard let url = components.url else { throw MessagesUIFailure.navigationUnavailable }
        MessagesUISession.activate()
        _ = try resetTaggedEditor("")
        try MessagesNavigation.open(url)
        RunLoop.current.run(until: Date().addingTimeInterval(3))
        let session = try uiSession(), root = session.root
        var budget = 4000
        let recipients = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXPopUpButtonRole }
        let descriptions = recipients.map { value($0, kAXDescriptionAttribute) }
        guard budget > 0, recipients.count == handles.count, Set(handles).count == handles.count, handles.allSatisfy({ handle in
            descriptions.filter { MessagesParityFixture.recipientMatches($0, handle: handle) }.count == 1
        }) else { throw MessagesUIFailure.recipientMismatch }
        budget = 4000
        let fields = matching(root, budget: &budget) { value($0, kAXIdentifierAttribute) == "messageBodyField" }
        guard fields.count == 1 else { throw MessagesUIFailure.composerUnavailable }
        guard textValue(fields[0]).isEmpty else { throw MessagesUIFailure.draftPresent }
        let placeholder = value(fields[0], kAXPlaceholderValueAttribute)
        guard placeholder.contains(args["service"].string) else { throw MessagesUIFailure.wrongService }
        try session.type(args["text"].string, into: fields[0], value: textValue)
        budget = 4000
        let finalRecipients = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXPopUpButtonRole }
        let finalDescriptions = finalRecipients.map { value($0, kAXDescriptionAttribute) }
        guard budget > 0, finalRecipients.count == handles.count, handles.allSatisfy({ handle in
            finalDescriptions.filter { MessagesParityFixture.recipientMatches($0, handle: handle) }.count == 1
        }), session.focused(fields[0]), textValue(fields[0]) == args["text"].string else { throw MessagesUIFailure.recipientMismatch }
        try session.press(MessagesUISession.enter)
    }
    static func perform(_ prepared: Prepared) throws -> JSON {
        let action = try MessagesAction(operation: prepared.action["op"].string)
        let args = prepared.action["messages"]
        if [.markRead, .markUnread].contains(action) { return try performReadState(prepared) }
        let session = try uiSession(), root = session.root
        let bubble = try target(root, prepared: prepared)
        let initialFocus = focus(root)
        let start = ContinuousClock.now
        let before = snapshot(root, prepared: prepared)
        try session.fence()
        guard focus(root) == initialFocus else { throw SourceFailure.conflict }
        let menu: String
        switch action {
        case .reply: menu = "Reply…"
        case .react: menu = args["reaction"].string == "emoji" ? args["emoji"].string : ["love": "Heart", "like": "Thumbs up", "dislike": "Thumbs down", "laugh": "Ha ha!", "emphasize": "Exclamation mark", "question": "Question mark"][args["reaction"].string] ?? ""
        case .edit: menu = "Edit"
        case .unsend: menu = "Undo Send"
        case .markRead: menu = "Mark as Read"
        case .markUnread: menu = "Mark as Unread"
        case .deleteMessage, .deleteChat: menu = "Delete…"
        case .renameGroup: menu = "Change Group Name"
        case .addMember: menu = "Add Member"
        case .removeMember: menu = "Remove from Conversation"
        case .leaveGroup: menu = "Leave this Conversation"
        default: throw SourceFailure.capabilityUnproven
        }
        try session.fence()
        guard start.duration(to: ContinuousClock.now) < .seconds(10) else { throw SourceFailure.conflict }
        var actionNames: CFArray?
        let copied = AXUIElementCopyActionNames(bubble, &actionNames)
        let normalized = menu.replacingOccurrences(of: "…", with: "")
        let direct = (actionNames as? [String] ?? []).filter {
            let title = String($0.split(separator: "\n").first ?? "").replacingOccurrences(of: "Name:", with: "").replacingOccurrences(of: "…", with: "")
            return title == normalized
        }
        if copied == .success && direct.count == 1 {
            guard AXUIElementPerformAction(bubble, direct[0] as CFString) == .success else { throw SourceFailure.conflict }
        } else {
            try contextMenu(bubble, session: session)
            try press(session, title: menu)
        }
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        switch action {
        case .edit: try edit(session, text: args["text"].string, previous: prepared.nativeState["target"]["text"].string)
        case .reply:
            try replace(session, text: args["text"].string)
            try session.press(MessagesUISession.enter)
        case .react: break
        case .deleteMessage, .deleteChat:
            do { try press(session, title: "Delete") }
            catch {
                try? press(session, title: "Cancel")
                throw error
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
            _ = try acknowledgeDeletedInfo()
        case .renameGroup: try replace(session, text: args["name"].string); try session.press(MessagesUISession.enter)
        case .addMember: try replace(session, text: args["handle"].string); try session.press(MessagesUISession.enter)
        default: break
        }
        let after = snapshot(root, prepared: prepared)
        try session.fence()
        return .object(["before_accessibility": before, "after_accessibility": after, "target_verified": .bool(true)])
    }
}
