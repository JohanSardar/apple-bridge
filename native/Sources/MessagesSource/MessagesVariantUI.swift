import Foundation
import AppKit
import ApplicationServices
import AVFoundation
import Carbon
import SourceSupport

@UIActor
enum MessagesVariantUI {
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
    static func uiSession() throws -> MessagesUISession {
        guard NSEvent.modifierFlags.intersection([.command,.control,.option,.shift]).isEmpty else { throw SourceFailure.conflict }
        return try MessagesUISession.frontmost()
    }
    static func press(_ element: AXUIElement) throws {
        guard AXUIElementPerformAction(element,kAXPressAction as CFString) == .success else { throw SendVariantFailure.automationDenied }
        RunLoop.current.run(until:Date().addingTimeInterval(0.15))
    }
    static func unique(_ elements: [AXUIElement], failure: SendVariantFailure) throws -> AXUIElement {
        guard elements.count==1 else { throw failure };return elements[0]
    }
    static func menu(_ title: String) throws {
        let application=try uiSession().root
        try press(try unique(find(application,role:kAXMenuBarItemRole,name:"Format"),failure:.formatMenuUnavailable))
        try press(try unique(find(application,role:kAXMenuItemRole,name:title),failure:.formatMenuUnavailable))
    }
    static func send(_ args: JSON, target: String, targetData: JSON = .null, expiresAt: Date = Date().addingTimeInterval(30)) throws {
        for bundle in ["com.apple.MobileSMS"] {
            let target=NSAppleEventDescriptor(bundleIdentifier:bundle)
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc,AEEventClass(kAECoreSuite),AEEventID(kAEGetData),false)==noErr else { throw SendVariantFailure.automationDenied }
        }
        let variant=args["variant"],kind=variant["kind"].string
        if kind=="audio" {
            guard AVCaptureDevice.authorizationStatus(for:.audio) == .authorized else { throw SendVariantFailure.audioConsentPending }
            throw SendVariantFailure.audioMessageFlagUnverified
        }
        var url=URLComponents();url.scheme="imessage";url.path="open";url.queryItems=[URLQueryItem(name:"message-guid",value:target)]
        guard let destination=url.url else { throw MessagesUIFailure.navigationUnavailable }
        try MessagesNavigation.open(destination)
        RunLoop.current.run(until:Date().addingTimeInterval(1))
        try MessagesAccessibility.validate(Prepared(nativeState:.object(["target":targetData]),fingerprint:"",action:.object(["op":.string("messages.send"),"native_key":.string(target)])))
        let session=try uiSession(),application=session.root,editor=try unique(find(application,identifier:"messageBodyField"),failure:.composerUnavailable)
        guard value(editor,kAXValueAttribute).isEmpty else { throw SendVariantFailure.draftPresent }
        var sendStarted=false
        defer {
            if !sendStarted,(try? session.fence()) != nil,value(editor,kAXValueAttribute)==args["text"].string,(try? session.focus(editor,failure:SendVariantFailure.composerUnavailable)) != nil {
                if (try? session.press(0,command:true)) != nil,session.focused(editor),value(editor,kAXValueAttribute)==args["text"].string { try? session.press(51) }
            }
        }
        try MessagesAccessibility.replace(session,text:args["text"].string)
        if kind=="schedule" {
            let buttons=find(application,role:kAXButtonRole).filter { ["Apps","Add","Show Apps"].contains(value($0,kAXDescriptionAttribute)) || ["AppsButton","AppDrawerButton"].contains(value($0,kAXIdentifierAttribute)) }
            try press(try unique(buttons,failure:.scheduleDateControlsUnverified))
            try press(try unique(find(application,name:"Send Later"),failure:.scheduleDateControlsUnverified))
            guard let date = parseTimestamp(variant["scheduled_for"].string) else { throw SendVariantFailure.invalidVariant }
            let fields = find(application, role: "AXDateField")
            guard !fields.isEmpty else { throw SendVariantFailure.scheduleDateControlsMissing }
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "MM/dd/yyyy h:mm a"
            let formatted = formatter.string(from: date)
            for field in fields {
                guard AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, date as NSDate) == .success || AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, formatted as CFString) == .success else { throw SendVariantFailure.scheduleDateControlsUnverified }
            }
            try session.press(MessagesUISession.enter)
        }
        if kind=="mention" {
            let own = Set(try MessagesProbes.discoverSelfHandles().handles.map(MessagesProbes.normalize))
            let expected = MessagesProbes.normalize(variant["handle_id"].string)
            guard own.contains(expected) else { throw SendVariantFailure.mentionSuggestionIdentityUnverified }
            let suggestions = find(application, role: kAXButtonRole).filter { element in
                [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, kAXHelpAttribute].contains { attribute in
                    let raw = value(element, attribute)
                    return MessagesProbes.normalize(raw) == expected || raw.components(separatedBy: CharacterSet(charactersIn: " ,\n<>" )).contains { MessagesProbes.normalize($0) == expected }
                }
            }
            try press(try unique(suggestions, failure: .mentionSuggestionMissing))
        }
        let labels=["bold":"Bold","italic":"Italic","underline":"Underline","strikethrough":"Strikethrough"]
        if kind=="formatting" {
            for entry in variant["formatting"].array {
                guard Date()<expiresAt else { throw SourceFailure.expired }
                var range=try MessagesSendVariants.range(entry,text:args["text"].string)
                guard let selected=AXValueCreate(.cfRange,&range),AXUIElementSetAttributeValue(editor,kAXSelectedTextRangeAttribute as CFString,selected) == .success else { throw SendVariantFailure.composerUnavailable }
                for style in entry["styles"].array.map(\.string) { guard let label=labels[style] else { throw SendVariantFailure.invalidVariant };try menu(label) }
                if !entry["effect"].string.isEmpty { try menu(entry["effect"].string.capitalized) }
            }
        } else if kind=="effect" {
            let buttons=find(application,role:kAXButtonRole).filter { element in ["Apps","Add","Show Apps"].contains(value(element,kAXDescriptionAttribute)) || ["AppsButton","AppDrawerButton"].contains(value(element,kAXIdentifierAttribute)) }
            try press(try unique(buttons,failure:.effectPickerUnavailable))
            try press(try unique(find(application,name:"Message Effects"),failure:.effectPickerUnavailable))
            let label=variant["effect"].string=="invisible-ink" ? "Invisible Ink":variant["effect"].string.capitalized
            try press(try unique(find(application,name:label),failure:.effectPickerUnavailable))
        } else if !["schedule", "mention"].contains(kind) { throw SendVariantFailure.invalidVariant }
        guard (try? session.fence()) != nil, kind == "mention" || value(editor,kAXValueAttribute)==args["text"].string else { throw SendVariantFailure.draftChanged }
        try MessagesAccessibility.validate(Prepared(nativeState:.object(["target":targetData]),fingerprint:"",action:.object(["op":.string("messages.send"),"native_key":.string(target)])))
        try session.focus(editor,failure:SendVariantFailure.composerUnavailable)
        guard Date()<expiresAt else { throw SourceFailure.expired }
        try session.fence()
        sendStarted=true
        guard session.driver.keys([MessagesUISession.enter], command: false)==0 else { throw SendVariantFailure.sendReceiptUnknown }
    }
}
