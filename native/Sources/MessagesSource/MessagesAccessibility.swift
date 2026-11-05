import Foundation
import AppKit
import ApplicationServices
import SourceSupport

@UIActor
public enum MessagesAccessibility {
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
}
