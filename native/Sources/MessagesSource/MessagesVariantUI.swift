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
    static func focused(_ root: AXUIElement, editor: AXUIElement) -> Bool {
        var current:CFTypeRef?
        guard AXUIElementCopyAttributeValue(root,kAXFocusedUIElementAttribute as CFString,&current) == .success,let current,CFGetTypeID(current)==AXUIElementGetTypeID() else { return false }
        return CFEqual(current,editor)
    }
    static func root() throws -> (AXUIElement,pid_t) {
        guard AXIsProcessTrusted(),NSEvent.modifierFlags.intersection([.command,.control,.option,.shift]).isEmpty,let app=MessagesNavigation.application,NSWorkspace.shared.frontmostApplication?.processIdentifier==app.processIdentifier else { throw SourceFailure.conflict }
        let root=AXUIElementCreateApplication(app.processIdentifier);AXUIElementSetMessagingTimeout(root,0.25);return (root,app.processIdentifier)
    }
    static func press(_ element: AXUIElement) throws {
        guard AXUIElementPerformAction(element,kAXPressAction as CFString) == .success else { throw SendVariantFailure.automationDenied }
        RunLoop.current.run(until:Date().addingTimeInterval(0.15))
    }
    static func unique(_ elements: [AXUIElement], failure: SendVariantFailure) throws -> AXUIElement {
        guard elements.count==1 else { throw failure };return elements[0]
    }
    static func menu(_ title: String) throws {
        let (root,_)=try root()
        try press(try unique(find(root,role:kAXMenuBarItemRole,name:"Format"),failure:.formatMenuUnavailable))
        try press(try unique(find(root,role:kAXMenuItemRole,name:title),failure:.formatMenuUnavailable))
    }
    static func send(_ args: JSON, target: String, targetData: JSON = .null, expiresAt: Date = Date().addingTimeInterval(30)) throws {
        if NSRunningApplication.runningApplications(withBundleIdentifier:"com.apple.systemevents").isEmpty {
            guard let url=NSWorkspace.shared.urlForApplication(withBundleIdentifier:"com.apple.systemevents") else { throw SendVariantFailure.automationDenied }
            let configuration=NSWorkspace.OpenConfiguration();configuration.activates=false
            NSWorkspace.shared.openApplication(at:url,configuration:configuration,completionHandler:nil)
            let deadline=Date().addingTimeInterval(2)
            while NSRunningApplication.runningApplications(withBundleIdentifier:"com.apple.systemevents").isEmpty,Date()<deadline { RunLoop.current.run(until:Date().addingTimeInterval(0.05)) }
        }
        for bundle in ["com.apple.MobileSMS","com.apple.systemevents"] {
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
        let (application,pid)=try root(),editor=try unique(find(application,identifier:"messageBodyField"),failure:.composerUnavailable)
        guard value(editor,kAXValueAttribute).isEmpty else { throw SendVariantFailure.draftPresent }
        var sendStarted=false
        defer {
            if !sendStarted,NSWorkspace.shared.frontmostApplication?.processIdentifier==pid,value(editor,kAXValueAttribute)==args["text"].string {
                if AXUIElementSetAttributeValue(editor,kAXFocusedAttribute as CFString,kCFBooleanTrue) == .success,focused(application,editor:editor) {
                    _=MessagesAppleScript.execute("tell application \"System Events\" to tell process \"Messages\" to key code 0 using {command down}")
                    if NSWorkspace.shared.frontmostApplication?.processIdentifier==pid,focused(application,editor:editor),value(editor,kAXValueAttribute)==args["text"].string { _=MessagesAppleScript.execute("tell application \"System Events\" to tell process \"Messages\" to key code 51") }
                }
            }
        }
        try MessagesAccessibility.replace(application,text:args["text"].string)
        if kind=="schedule" {
            let buttons=find(application,role:kAXButtonRole).filter { ["Apps","Add","Show Apps"].contains(value($0,kAXDescriptionAttribute)) || ["AppsButton","AppDrawerButton"].contains(value($0,kAXIdentifierAttribute)) }
            try press(try unique(buttons,failure:.scheduleDateControlsUnverified))
            try press(try unique(find(application,name:"Send Later"),failure:.scheduleDateControlsUnverified))
            guard !find(application,role:"AXDateField").isEmpty else { throw SendVariantFailure.scheduleDateControlsMissing }
            throw SendVariantFailure.scheduleDateControlsUnverified
        }
        if kind=="mention" {
            let own=try MessagesProbes.discoverSelfHandles().handles
            let suggestions=own.flatMap { find(application,name:$0) }
            guard !suggestions.isEmpty else { throw SendVariantFailure.mentionSuggestionMissing }
            throw SendVariantFailure.mentionSuggestionIdentityUnverified
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
        } else { throw SendVariantFailure.invalidVariant }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier==pid,value(editor,kAXValueAttribute)==args["text"].string else { throw SendVariantFailure.draftChanged }
        try MessagesAccessibility.validate(Prepared(nativeState:.object(["target":targetData]),fingerprint:"",action:.object(["op":.string("messages.send"),"native_key":.string(target)])))
        guard AXUIElementSetAttributeValue(editor,kAXFocusedAttribute as CFString,kCFBooleanTrue) == .success,focused(application,editor:editor) else { throw SendVariantFailure.composerUnavailable }
        guard Date()<expiresAt else { throw SourceFailure.expired }
        sendStarted=true
        guard MessagesAppleScript.execute("tell application \"System Events\" to tell process \"Messages\" to key code 36")==0 else { throw SendVariantFailure.sendReceiptUnknown }
    }
}
