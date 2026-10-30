#if LAB
import Foundation
import AppKit
import Carbon
import CryptoKit
import ApplicationServices
import SourceSupport

public enum SafariCleanup {
    @MainActor
    public static func run(_ request: JSON) async throws -> JSON {
        let run = request["run_id"].string
        guard run.range(of: #"^[A-Za-z0-9_-]+$"#, options: .regularExpression) != nil else { throw SourceFailure.invalid }
        let tabs = SafariTabs()
        guard await tabs.permitted(fresh: true), AXIsProcessTrusted() else { throw SourceFailure.permissionMissing }
        let tag = "[ab-test " + run + "]"
        let url = "https://example.com/?ab-test=" + run
        func reading() throws -> [JSON] {
            let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Safari/Bookmarks.plist")
            let root = try PropertyListSerialization.propertyList(from: Data(contentsOf: path), format: nil)
            func walk(_ node: Any, _ reading: Bool) throws -> [JSON] {
                guard let node = node as? [String: Any] else { throw SourceFailure.corrupt }
                if let children = node["Children"] as? [Any] {
                    return try children.flatMap { try walk($0, reading || node["Title"] as? String == "com.apple.ReadingList") }
                }
                guard reading, let id = node["WebBookmarkUUID"] as? String, let value = node["URLString"] as? String else { return [] }
                return [.object(["ID": .string("reading:" + id), "Tag": .string(value == url ? tag : ""), "Container": .string("reading:" + SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined())])]
            }
            return try walk(root, false)
        }
        let currentTabs = try await tabs.tabs()
        var items = try reading()
        items += currentTabs.map { row in
            let value = row["data"]["url"].string
            return .object(["ID": .string("tab:" + row["native_key"].string), "Tag": .string(value == url ? tag : ""), "Container": .string("tab:" + SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined())])
        }
        if request["action"].string == "snapshot" { return .object(["items": .array(items)]) }
        let item = request["item"]
        guard item["Tag"].string == tag, items.contains(item) else { throw SourceFailure.conflict }
        if item["ID"].string.hasPrefix("tab:") {
            let key = String(item["ID"].string.dropFirst(4)).split(separator: ":")
            guard key.count == 2, let window = Int(key[0]), let index = Int(key[1]), index > 0 else { throw SourceFailure.invalid }
            var error: NSDictionary?
            let body = "tell application id \"com.apple.Safari\"\nif URL of tab \(index) of window id \(window) is not \"\(url)\" then error number -1700\nclose tab \(index) of window id \(window)\nend tell"
            _ = NSAppleScript(source: body)?.executeAndReturnError(&error)
            guard error == nil else { throw SourceFailure.unavailable }
        } else {
            guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.Safari").first else { throw SourceFailure.permissionMissing }
            let root = AXUIElementCreateApplication(app.processIdentifier)
            func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
                var result: CFTypeRef?
                guard AXUIElementCopyAttributeValue(element, name as CFString, &result) == .success else { return nil }
                return result
            }
            func menuItems(_ titles: Set<String>) -> [AXUIElement] {
                var count = 0
                func walk(_ element: AXUIElement, _ depth: Int) -> [AXUIElement] {
                    count += 1
                    guard depth < 30, count < 10000 else { return [] }
                    let title = attribute(element, kAXTitleAttribute) as? String ?? ""
                    let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                    var result = role == "AXMenuItem" && titles.contains(title) ? [element] : []
                    for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += walk(child, depth + 1) }
                    return result
                }
                return walk(root, 0)
            }
            let showReading = menuItems(["Show Reading List Sidebar", "Show Reading List"])
            let bookmarkVisible = !menuItems(["Hide Bookmarks Sidebar"]).isEmpty
            var restoreSidebar = false
            if showReading.count == 1 {
                guard AXUIElementPerformAction(showReading[0], kAXPressAction as CFString) == .success else { throw SourceFailure.capabilityUnproven }
                restoreSidebar = true
                try await Task.sleep(for: .milliseconds(200))
            }
            func restore() throws {
                guard restoreSidebar else { return }
                let titles: Set<String> = bookmarkVisible ? ["Show Bookmarks Sidebar"] : ["Hide Reading List Sidebar", "Hide Sidebar", "Hide Reading List"]
                let items = menuItems(titles)
                guard items.count == 1, AXUIElementPerformAction(items[0], kAXPressAction as CFString) == .success else { throw SourceFailure.capabilityUnproven }
                restoreSidebar = false
            }
            defer { try? restore() }
            func rowURLs(_ element: AXUIElement, _ depth: Int) -> Set<String> {
                guard depth < 12 else { return [] }
                let value = attribute(element, "AXURL")
                let url = (value as? URL)?.absoluteString ?? value as? String ?? ""
                var result: Set<String> = url.isEmpty ? [] : [url]
                for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result.formUnion(rowURLs(child, depth + 1)) }
                return result
            }
            var visited = 0
            func matching(_ element: AXUIElement, _ depth: Int) -> [AXUIElement] {
                visited += 1
                guard depth < 30, visited < 10000 else { return [] }
                let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                var result = role == "AXRow" && rowURLs(element, 0) == [url] ? [element] : []
                for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += matching(child, depth + 1) }
                return result
            }
            var rows = matching(root, 0)
            var searchField: AXUIElement?
            var priorSearch = ""
            defer { if let searchField { _ = AXUIElementSetAttributeValue(searchField, kAXValueAttribute as CFString, priorSearch as CFString) } }
            if rows.isEmpty {
                func searchFields(_ element: AXUIElement, _ depth: Int) -> [AXUIElement] {
                    guard depth < 30 else { return [] }
                    var result = (attribute(element, kAXIdentifierAttribute) as? String) == "ReadingListSearchField" ? [element] : []
                    for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += searchFields(child, depth + 1) }
                    return result
                }
                let fields = searchFields(root, 0)
                guard fields.count == 1, items.filter({ $0["ID"].string.hasPrefix("reading:") && $0["Tag"].string == tag }).count == 1 else { throw SourceFailure.capabilityUnproven }
                searchField = fields[0]
                priorSearch = attribute(fields[0], kAXValueAttribute) as? String ?? ""
                guard AXUIElementSetAttributeValue(fields[0], kAXValueAttribute as CFString, url as CFString) == .success else { throw SourceFailure.capabilityUnproven }
                try await Task.sleep(for: .milliseconds(300))
                guard attribute(fields[0], kAXValueAttribute) as? String == url else { throw SourceFailure.capabilityUnproven }
                func visibleRows(_ element: AXUIElement, _ depth: Int) -> [AXUIElement] {
                    guard depth < 30 else { return [] }
                    let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                    var result = role == "AXRow" && !(attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).isEmpty ? [element] : []
                    for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += visibleRows(child, depth + 1) }
                    return result
                }
                rows = visibleRows(root, 0)
            }
            guard rows.count == 1, AXUIElementPerformAction(rows[0], kAXShowMenuAction as CFString) == .success else { throw SourceFailure.capabilityUnproven }
            try await Task.sleep(for: .milliseconds(200))
            func removeMenu(_ element: AXUIElement, _ depth: Int) -> [AXUIElement] {
                guard depth < 30 else { return [] }
                let title = attribute(element, kAXTitleAttribute) as? String ?? ""
                let role = attribute(element, kAXRoleAttribute) as? String ?? ""
                var result = role == "AXMenuItem" && title == "Remove Item" ? [element] : []
                for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] { result += removeMenu(child, depth + 1) }
                return result
            }
            let menus = removeMenu(root, 0)
            guard menus.count == 1, AXUIElementPerformAction(menus[0], kAXPressAction as CFString) == .success else { throw SourceFailure.capabilityUnproven }
            try restore()
        }
        return .object(["passed": .bool(true)])
    }
}
#endif
