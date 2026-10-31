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
    static func target(_ root: AXUIElement, prepared: Prepared) throws -> AXUIElement {
        let guid = prepared.action["native_key"].string
        guard !guid.isEmpty else { throw SourceFailure.capabilityUnproven }
        var budget = 4000
        let expected = prepared.nativeState["target"]["text"].string
        let mediaNames = prepared.nativeState["cleanup_authorized"].bool ? prepared.nativeState["target"]["cleanup_media_names"].array.map(\.string) : []
        guard !expected.isEmpty || !mediaNames.isEmpty else { throw SourceFailure.conflict }
        let outgoing = prepared.nativeState["target"]["is_from_me"].bool
        if prepared.nativeState["conversation_target"].bool {
            let rows = matching(root, budget: &budget) { element in
                guard value(element, kAXRoleAttribute) == kAXRowRole else { return false }
                var textBudget = 100
                return matching(element, budget: &textBudget) { node in [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains { [expected, "You: " + expected].contains(value(node, $0)) } }.count > 0 && textBudget > 0
            }
            guard budget > 0, rows.count == 1 else { throw MessagesUIFailure.targetUnavailable }
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
            guard budget > 0, let index = MessagesMediaCleanupProof.targetIndex(candidates, names: Set(mediaNames), outgoing: outgoing, allowSelfEcho: prepared.nativeState["target"]["cleanup_self_echo"].bool, equivalentOwned: prepared.nativeState["target"]["cleanup_equivalent_owned_media"].bool) else { throw MessagesUIFailure.targetUnavailable }
            return bubbles[index]
        }
        let targets = matching(root, budget: &budget) { element in
            let identifier = value(element, kAXIdentifierAttribute)
            let description = value(element, kAXDescriptionAttribute)
            var textBudget = 100
            let exactText = identifier == "Sticker" && matching(element, budget: &textBudget) { value($0, kAXIdentifierAttribute) == "CKBalloonTextView" && textValue($0) == expected }.count == 1
            return exactText && (prepared.nativeState["cleanup_authorized"].bool || description.hasPrefix(outgoing ? "Your iMessage," : "iMessage,"))
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
        return targets[0]
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
    static func press(_ session: MessagesUISession, title: String) throws {
        let deadline = Date().addingTimeInterval(2)
        repeat {
            var budget = 4000
            let windows = children(session.root).filter { value($0, kAXRoleAttribute) == kAXWindowRole }
            let matches = windows.flatMap { window in matching(window, budget: &budget) {
                [kAXMenuItemRole, kAXButtonRole].contains(value($0, kAXRoleAttribute)) && ([title, title.replacingOccurrences(of: "…", with: "")].contains(value($0, kAXTitleAttribute)) || value($0, kAXDescriptionAttribute) == title)
            } }
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
                guard previous.isEmpty || previous.contains("[ab-test ") else { throw MessagesUIFailure.draftPresent }
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
    private static func resetTaggedEditor() throws -> Bool {
        let session = try uiSession(), root = session.root
        var budget = 2000
        let cancel = matching(root, budget: &budget) { value($0, kAXDescriptionAttribute) == "Cancel edit" }
        guard cancel.count == 1 else { return false }
        budget = 2000
        let fields = matching(root, budget: &budget) { value($0, kAXRoleAttribute) == kAXTextFieldRole && value($0, kAXIdentifierAttribute) != "messageBodyField" && textValue($0).range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil }
        guard !fields.isEmpty else { return false }
        guard AXUIElementPerformAction(cancel[0], kAXPressAction as CFString) == .success else {
            try? PrivateReport.write(["stage": "cancel_tagged_editor"], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-prepare-diagnostic.json").path))
            throw SourceFailure.conflict
        }
        try session.press(MessagesUISession.escape)
        RunLoop.current.run(until: Date().addingTimeInterval(0.35))
        return true
    }
    static func openMessage(_ guid: String, settle: TimeInterval = 3, resetEditor: Bool = true) throws {
        var components = URLComponents()
        components.scheme = "imessage"
        components.path = "open"
        components.queryItems = [URLQueryItem(name: "message-guid", value: guid)]
        guard let url = components.url else { throw MessagesUIFailure.navigationUnavailable }
        MessagesUISession.activate()
        if resetEditor {
            _ = try resetTaggedEditor()
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
        _ = try resetTaggedEditor()
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
        var actionNames: CFArray?
        let copied = AXUIElementCopyActionNames(bubble, &actionNames)
        let normalized = menu.replacingOccurrences(of: "…", with: "")
        let direct = (actionNames as? [String] ?? []).filter {
            let title = String($0.split(separator: "\n").first ?? "").replacingOccurrences(of: "Name:", with: "").replacingOccurrences(of: "…", with: "")
            return title == normalized
        }
        if action != .reply && copied == .success && direct.count == 1 {
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
            try press(session, title: "Delete")
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
            _ = try acknowledgeDeletedInfo()
        case .renameGroup: try replace(session, text: args["name"].string); try session.press(MessagesUISession.enter)
        case .addMember: try replace(session, text: args["handle"].string); try session.press(MessagesUISession.enter)
        default: break
        }
        let after = snapshot(root, prepared: prepared)
        try session.fence()
        guard start.duration(to: ContinuousClock.now) < .seconds(10) else { throw SourceFailure.conflict }
        return .object(["before_accessibility": before, "after_accessibility": after, "target_verified": .bool(true)])
    }
}
