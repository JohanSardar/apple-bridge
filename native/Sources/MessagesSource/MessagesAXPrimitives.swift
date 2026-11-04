import Foundation
import AppKit
import ApplicationServices
import SourceSupport

@UIActor
enum MessagesAXPrimitives {
    private static var elements: [Int: AXUIElement] = [:]
    private static var nextHandle = 1
    private static var previewKnown: [AXUIElement] = []

    private static func handle(_ element: AXUIElement) -> Int {
        if elements.count > 20000 { elements.removeAll() }
        let handle = nextHandle
        nextHandle += 1
        elements[handle] = element
        return handle
    }

    private static func element(_ args: JSON) throws -> AXUIElement {
        guard let element = elements[Int(args["handle"].number)] else { throw SourceFailure.conflict }
        return element
    }

    private static func node(_ element: AXUIElement, attributes: [String]) -> JSON {
        var attrs: [String: JSON] = [:]
        for attribute in attributes {
            let value = MessagesAccessibility.value(element, attribute)
            if !value.isEmpty { attrs[attribute] = .string(value) }
        }
        return .object(["handle": .number(Double(handle(element))), "attrs": .object(attrs)])
    }

    private static func walk(_ root: AXUIElement, match: [String: Set<String>], attributes: [String], budget: inout Int, depth: Int?, menuBar: Bool, visit: (AXUIElement) -> JSON) -> [JSON] {
        guard budget > 0 else { return [] }
        if !menuBar, MessagesAccessibility.value(root, kAXRoleAttribute) == kAXMenuBarRole { return [] }
        budget -= 1
        var found: [JSON] = []
        if match.allSatisfy({ $0.value.contains(MessagesAccessibility.value(root, $0.key)) }) { found.append(visit(root)) }
        if depth.map({ $0 > 0 }) ?? true {
            for child in MessagesAccessibility.children(root) {
                found += walk(child, match: match, attributes: attributes, budget: &budget, depth: depth.map { $0 - 1 }, menuBar: menuBar, visit: visit)
            }
        }
        return found
    }

    private static func matcher(_ value: JSON) -> [String: Set<String>] {
        value.object.mapValues { Set($0.array.map(\.string)) }
    }

    private static func find(_ args: JSON) throws -> JSON {
        let root = args["within"] == .null ? try MessagesAccessibility.uiSession().root : try element(.object(["handle": args["within"]]))
        var budget = Int(args["budget"].number)
        let attributes = args["attributes"].array.map(\.string)
        let nested = args["nested"]
        let nodes = walk(root, match: matcher(args["match"]), attributes: attributes, budget: &budget, depth: args["depth"] == .null ? nil : Int(args["depth"].number), menuBar: args["menu_bar"].bool) { element in
            var fields = node(element, attributes: attributes).object
            if nested != .null {
                var inner = Int(nested["budget"].number)
                let names = nested["attributes"].array.map(\.string)
                let found = walk(element, match: matcher(nested["match"]), attributes: names, budget: &inner, depth: nil, menuBar: false) { node($0, attributes: names) }
                fields["nested"] = .object(["nodes": .array(found), "budget_left": .number(Double(inner))])
            }
            return .object(fields)
        }
        return .object(["nodes": .array(nodes), "budget_left": .number(Double(budget))])
    }

    private static func previewWindows(_ session: MessagesUISession) -> [AXUIElement] {
        let services = NSWorkspace.shared.runningApplications.filter { $0.bundleIdentifier == "com.apple.quicklook.QuickLookUIService" }.map { AXUIElementCreateApplication($0.processIdentifier) }
        return ([session.root] + services).flatMap { root in
            var windows: CFTypeRef?
            AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &windows)
            return windows as? [AXUIElement] ?? MessagesAccessibility.children(root).filter { MessagesAccessibility.value($0, kAXRoleAttribute) == kAXWindowRole }
        }
    }

    private static func previewValues(_ budgetValue: Int) throws -> JSON {
        let session = try MessagesAccessibility.uiSession()
        let windows = previewWindows(session).filter { window in !previewKnown.contains(where: { CFEqual($0, window) }) }
        var budget = budgetValue
        var values: Set<String> = []
        for window in windows {
            _ = MessagesAccessibility.matching(window, budget: &budget) { node in
                var attributes: CFArray?
                AXUIElementCopyAttributeNames(node, &attributes)
                for attribute in attributes as? [String] ?? [] {
                    let text = MessagesAccessibility.value(node, attribute)
                    if !text.isEmpty { values.insert(text) }
                }
                return false
            }
        }
        return .object(["values": .array(values.sorted().map(JSON.string))])
    }

    static func answer(_ operation: String, args: JSON) throws -> JSON {
        switch operation {
        case "messages.ax_state":
            let session = CGSessionCopyCurrentDictionary() as? [String: Any]
            return .object(["trusted": .bool(AXIsProcessTrusted()), "screen_locked": .bool(session?["CGSSessionScreenIsLocked"] as? Bool == true)])
        case "messages.ax_activate":
            MessagesUISession.activate()
        case "messages.ax_open_url":
            guard let url = URL(string: args["url"].string) else { throw MessagesUIFailure.navigationUnavailable }
            try MessagesNavigation.open(url)
        case "messages.ax_find":
            return try find(args)
        case "messages.ax_value":
            return .object(["value": .string(MessagesAccessibility.value(try element(args), args["attribute"].string))])
        case "messages.ax_actions":
            var actions: CFArray?
            guard AXUIElementCopyActionNames(try element(args), &actions) == .success else { throw SourceFailure.conflict }
            return .object(["actions": .array((actions as? [String] ?? []).map(JSON.string))])
        case "messages.ax_action":
            guard AXUIElementPerformAction(try element(args), args["action"].string as CFString) == .success else { throw SourceFailure.conflict }
        case "messages.ax_fence":
            try MessagesAccessibility.uiSession().fence()
        case "messages.ax_key":
            try MessagesAccessibility.uiSession().press(Int(args["code"].number))
        case "messages.ax_focused":
            var focused: CFTypeRef?
            let root = try MessagesAccessibility.uiSession().root
            guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { return .object(["node": .null]) }
            return .object(["node": node(unsafeDowncast(focused, to: AXUIElement.self), attributes: args["attributes"].array.map(\.string))])
        case "messages.ax_replace_text":
            try MessagesAccessibility.uiSession().enter(args["text"].string, into: try element(args))
        case "messages.ax_clear_text":
            try MessagesAccessibility.uiSession().erase(try element(args))
        case "messages.ax_context_menu":
            try MessagesAccessibility.contextMenu(try element(args), session: try MessagesAccessibility.uiSession())
        case "messages.ax_preview_begin":
            previewKnown = previewWindows(try MessagesAccessibility.uiSession())
        case "messages.ax_preview_values":
            return try previewValues(Int(args["budget"].number))
        default:
            throw SourceFailure.unsupported
        }
        return .object([:])
    }
}
