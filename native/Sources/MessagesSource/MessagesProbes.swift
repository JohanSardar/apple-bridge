import Foundation
import Decoders
import Contacts
import AppKit
import ApplicationServices

public enum MessagesProbes {
    public static func meCard() -> [String] {
        guard CNContactStore.authorizationStatus(for: .contacts) == .authorized,
              let contact = try? CNContactStore().unifiedMeContactWithKeys(toFetch: [CNContactPhoneNumbersKey as CNKeyDescriptor, CNContactEmailAddressesKey as CNKeyDescriptor]) else { return [] }
        return contact.phoneNumbers.map { $0.value.stringValue } + contact.emailAddresses.map { $0.value as String }
    }
    public static func normalize(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["iMessage:", "SMS:", "RCS:", "mailto:", "tel:", "P:", "E:"] {
            if text.hasPrefix(prefix) { text.removeFirst(prefix.count) }
        }
        if text.contains("@") { return text.lowercased() }
        return text.filter { $0.isNumber || $0 == "+" }
    }

    #if LAB

    public static func run(_ arguments: [String]) throws -> [String: Any] {
        switch arguments[1] {
        case "rich-counts":
            return try richCounts()
        case "keychain-state":
            return try keychainState()
        case "session-state":
            var report: [String: Any] = ["accessibility": AXIsProcessTrusted(), "messages_frontmost": NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.MobileSMS", "messages_running": !NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").isEmpty]
            let system = AXUIElementCreateSystemWide()
            AXUIElementSetMessagingTimeout(system, 3)
            var focused: CFTypeRef?
            let focusError = AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute as CFString, &focused)
            report["focused_application_error"] = focusError.rawValue
            if let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() {
                let element = unsafeDowncast(focused, to: AXUIElement.self)
                var pid: pid_t = 0
                AXUIElementGetPid(element, &pid)
                let bundle = NSRunningApplication(processIdentifier: pid)?.bundleIdentifier ?? "unknown"
                report["focused_application"] = ["com.apple.MobileSMS", "com.apple.loginwindow", "com.apple.SecurityAgent"].contains(bundle) ? bundle : "other"
            }
            if let app = MessagesNavigation.application {
                let root = AXUIElementCreateApplication(app.processIdentifier)
                AXUIElementSetMessagingTimeout(root, 3)
                var role: CFTypeRef?
                report["messages_role_error"] = AXUIElementCopyAttributeValue(root,kAXRoleAttribute as CFString,&role).rawValue
                var windows: CFTypeRef?
                report["messages_windows_error"] = AXUIElementCopyAttributeValue(root,kAXWindowsAttribute as CFString,&windows).rawValue
                report["messages_window_count"] = (windows as? [AXUIElement])?.count ?? 0
            }
            return report
        default:
            throw ProbeFailure.invalidArguments
        }
    }

    private static func keychainState() throws -> [String: Any] {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-c", "import ctypes,json,subprocess,pathlib; p=subprocess.check_output(['/usr/bin/security','default-keychain','-d','user'],text=True).strip().strip(chr(34)); s=ctypes.CDLL('/System/Library/Frameworks/Security.framework/Security'); s.SecKeychainSetUserInteractionAllowed.argtypes=[ctypes.c_bool]; s.SecKeychainSetUserInteractionAllowed(False); s.SecKeychainGetStatus.argtypes=[ctypes.c_void_p,ctypes.POINTER(ctypes.c_uint32)]; v=ctypes.c_uint32(); code=s.SecKeychainGetStatus(None,ctypes.byref(v)); print(json.dumps({'default_is_login':pathlib.Path(p).name in ['login.keychain-db','login.keychain'],'status_code':code,'unlocked':bool(v.value & 1),'readable':bool(v.value & 2),'writable':bool(v.value & 4)}))"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0, let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw ProbeFailure.unavailable }
        return result
    }

    private static func decodedProbeText(_ row: [String: String]) -> String? {
        if let text = row["text"] { return text }
        guard let hex = row["body_hex"], hex.count.isMultiple(of: 2) else { return nil }
        var bytes = Data()
        var index = hex.startIndex
        while index < hex.endIndex {
            let end = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<end], radix: 16) else { return nil }
            bytes.append(byte)
            index = end
        }
        return TypedStream.decode(bytes).text
    }

    private static func accessibilityNode(_ element: AXUIElement, depth: Int, count: inout Int, raw: Bool) -> [String: Any] {
        count += 1
        var node: [String: Any] = [:]
        for attribute in [kAXRoleAttribute, kAXSubroleAttribute, kAXIdentifierAttribute, kAXDescriptionAttribute, kAXTitleAttribute, kAXValueAttribute, kAXHelpAttribute, kAXPlaceholderValueAttribute] {
            var value: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success, let text = (value as? String) ?? (value as? NSAttributedString)?.string {
                node[attribute] = raw || attribute == kAXRoleAttribute || attribute == kAXSubroleAttribute || attribute == kAXIdentifierAttribute ? text as Any : PrivateReport.redact(text) as Any
            }
        }
        if raw {
            for attribute in [kAXEnabledAttribute, kAXFocusedAttribute, "AXHidden"] {
                var value: CFTypeRef?
                if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success, let value = value as? NSNumber { node[attribute] = value.boolValue }
            }
            for attribute in [kAXPositionAttribute, kAXSizeAttribute] {
                var value: CFTypeRef?
                if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success, let value, CFGetTypeID(value) == AXValueGetTypeID() {
                    let ax = unsafeDowncast(value, to: AXValue.self)
                    var point = CGPoint.zero
                    var size = CGSize.zero
                    if AXValueGetValue(ax, .cgPoint, &point) { node[attribute] = [point.x, point.y] }
                    else if AXValueGetValue(ax, .cgSize, &size) { node[attribute] = [size.width, size.height] }
                }
            }
        }
        var actions: CFArray?
        if AXUIElementCopyActionNames(element, &actions) == .success {
            node["actions"] = (actions as? [String]) ?? []
        }
        if depth < 35 && count < 4000 {
            var children: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children) == .success, let elements = children as? [AXUIElement] {
                var result: [[String: Any]] = []
                for child in elements {
                    if count >= 4000 { break }
                    result.append(accessibilityNode(child, depth: depth + 1, count: &count, raw: raw))
                }
                node["children"] = result
            }
        }
        return node
    }
    #endif
}
