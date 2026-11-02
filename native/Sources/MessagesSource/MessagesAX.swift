import Foundation
import AppKit
import ApplicationServices
import Carbon
import SourceSupport

@UIActor
enum MessagesAX {
    static func value(_ element: AXUIElement, _ attribute: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element,attribute as CFString,&result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? ""
    }
    static func find(_ root: AXUIElement, role: String? = nil, name: String? = nil, identifier: String? = nil) -> [AXUIElement] {
        var budget=5000
        @UIActor func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget>0 else { return [] };budget-=1
            var result:[AXUIElement]=[]
            let actualRole=value(element,kAXRoleAttribute),title=value(element,kAXTitleAttribute),description=value(element,kAXDescriptionAttribute),actualIdentifier=value(element,kAXIdentifierAttribute)
            var matches=true
            if let role,actualRole != role { matches=false }
            if let name,title != name,description != name { matches=false }
            if let identifier,actualIdentifier != identifier { matches=false }
            if matches { result.append(element) }
            var children:CFTypeRef?;AXUIElementCopyAttributeValue(element,kAXChildrenAttribute as CFString,&children)
            for child in children as? [AXUIElement] ?? [] { result += walk(child) }
            return result
        }
        return walk(root)
    }
    static func press(_ element: AXUIElement) throws {
        guard AXUIElementPerformAction(element,kAXPressAction as CFString) == .success else { throw SourceFailure.permissionMissing }
        RunLoop.current.run(until:Date().addingTimeInterval(0.15))
    }
}
