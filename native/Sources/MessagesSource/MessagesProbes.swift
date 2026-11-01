import Foundation
import Decoders
import Contacts
import AppKit
import ApplicationServices

public enum MessagesProbes {
    public static func discoverSelfHandles() throws -> (handles: [String], source: String) {
        var discovered: Set<String> = []
        if CNContactStore.authorizationStatus(for: .contacts) == .authorized {
            let store = CNContactStore()
            if let contact = try? store.unifiedMeContactWithKeys(toFetch: [CNContactPhoneNumbersKey as CNKeyDescriptor, CNContactEmailAddressesKey as CNKeyDescriptor]) {
                let handles = contact.phoneNumbers.map { $0.value.stringValue } + contact.emailAddresses.map { $0.value as String }
                let normalized = Array(Set(handles.map(normalize).filter { !$0.isEmpty })).sorted()
                discovered.formUnion(normalized)
            }
        }
        let database = try MessagesDatabase()
        let columns = Set(try database.rows("PRAGMA table_info(message)").compactMap { $0["name"] })
        let available = ["destination_caller_id", "account_login"].filter { columns.contains($0) }
        guard !available.isEmpty else {
            guard !discovered.isEmpty else {throw ProbeFailure.noSelfHandle}
            return (discovered.sorted(),"contacts_me")
        }
        var handles: Set<String> = discovered
        for column in available {
            for row in try database.rows("SELECT DISTINCT \(column) AS handle FROM message WHERE is_from_me = 1 AND \(column) IS NOT NULL") {
                if let raw = row["handle"] {
                    let value = normalize(raw)
                    if !value.isEmpty { handles.insert(value) }
                }
            }
        }
        guard !handles.isEmpty else { throw ProbeFailure.noSelfHandle }
        return (handles.sorted(), discovered.isEmpty ? "messages_own_account" : "contacts_me_and_messages_own_account")
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
    private static var handlesURL: URL {
        PrivateReport.directory.appendingPathComponent("self-handles.json")
    }

    private static func selfHandle(_ arguments: [String]) throws -> String {
        let index: Int
        if let position = arguments.firstIndex(of: "--handle-index"), position + 1 < arguments.count, let value = Int(arguments[position + 1]) {
            index = value
        } else if arguments[1] == "ax-dump" {
            index = 0
        } else {
            throw ProbeFailure.invalidArguments
        }
        let data = try Data(contentsOf: PrivateReport.validate(handlesURL.path))
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let handles = object["handles"] as? [String], handles.indices.contains(index) else { throw ProbeFailure.noSelfHandle }
        let handle = handles[index]
        guard try discoverSelfHandles().handles.contains(handle) else { throw ProbeFailure.noSelfHandle }
        return handle
    }

    public static func run(_ arguments: [String]) throws -> [String: Any] {
        switch arguments[1] {
        case "parity-route":
            let handles = try discoverSelfHandles().handles
            let database = try MessagesDatabase()
            let unused = try MessagesSelfTestIsolation.unused(handles, database: database)
            return ["os_build": ProcessInfo.processInfo.operatingSystemVersionString, "self_handle_count": handles.count, "unused_self_handle_count": unused.count, "accessibility": AXIsProcessTrusted()]
        case "rich-counts":
            return try richCounts()
        case "dismiss-test-delete":
            guard AXIsProcessTrusted(), let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").first else { throw ProbeFailure.unavailable }
            func buttons(_ element: AXUIElement) -> [AXUIElement] {
                var title: CFTypeRef?
                var role: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &title)
                AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &role)
                if title as? String == "Cancel", role as? String == kAXButtonRole { return [element] }
                var children: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
                return (children as? [AXUIElement] ?? []).flatMap(buttons)
            }
            let matches = buttons(AXUIElementCreateApplication(app.processIdentifier))
            guard matches.count == 1 else { return ["cancel_count": matches.count] }
            var position: CFTypeRef?
            var size: CFTypeRef?
            AXUIElementCopyAttributeValue(matches[0], kAXPositionAttribute as CFString, &position)
            AXUIElementCopyAttributeValue(matches[0], kAXSizeAttribute as CFString, &size)
            guard let position, let size else { throw ProbeFailure.unavailable }
            var origin = CGPoint.zero
            var dimensions = CGSize.zero
            AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin)
            AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions)
            let x = Int(origin.x + dimensions.width / 2)
            let y = Int(origin.y + dimensions.height / 2)
            let code = MessagesAppleScript.execute(MessagesUIScript.click(CGPoint(x: x, y: y)))
            return ["apple_event_error": code, "cancel_count": matches.count]
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
        case "self-handles":
            let result = try discoverSelfHandles()
            let session = CGSessionCopyCurrentDictionary() as? [String: Any]
            let unused = try MessagesSelfTestIsolation.unused(result.handles, database: MessagesDatabase())
            let report: [String: Any] = ["handles": result.handles, "unused_handles": unused, "source": result.source, "index_base": 0, "cleanup_accessibility": AXIsProcessTrusted(), "cleanup_unlocked": session != nil && session?["CGSSessionScreenIsLocked"] as? Bool != true]
            try PrivateReport.write(report, to: PrivateReport.validate(handlesURL.path))
            return report
        case "send-self":
            return try sendSelf(arguments)
        case "ax-dump":
            return try dumpAccessibility(arguments)
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

    private static func sendSelf(_ arguments: [String]) throws -> [String: Any] {
        let handle = try selfHandle(arguments)
        guard let position = arguments.firstIndex(of: "--text"), position + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
        let text = arguments[position + 1]
        guard text.range(of: #"^\[ab-test [A-Za-z0-9_-]+\] .+"#, options: .regularExpression) != nil, text.utf8.count <= 4096 else { throw ProbeFailure.invalidArguments }
        let database = try MessagesDatabase()
        let baseline = try database.rows("SELECT COALESCE(MAX(ROWID), 0) AS maximum FROM message").first?["maximum"] ?? "0"
        let chats = try database.rows("SELECT DISTINCT c.guid FROM chat c JOIN chat_handle_join j ON c.ROWID=j.chat_id JOIN handle h ON h.ROWID=j.handle_id WHERE h.id=? AND c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join peers WHERE peers.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC", bindings: [handle])
        let rawGUID = chats.first?["guid"]
        let sendToChat: (String) -> String = { chatID in
            "tell application id \"com.apple.MobileSMS\" to send \(MessagesAppleScript.literal(text)) to chat id \(MessagesAppleScript.literal(chatID))"
        }
        let sendToBuddy = "tell application id \"com.apple.MobileSMS\"\nset targetService to first service whose service type = iMessage\nsend \(MessagesAppleScript.literal(text)) to buddy \(MessagesAppleScript.literal(handle)) of targetService\nend tell"
        var routes: [(name: String, script: String)] = []
        if let rawGUID {
            routes.append(("chat_id_raw", sendToChat(rawGUID)))
        } else {
            routes.append(("imessage_buddy", sendToBuddy))
        }
        var attempts: [[String: Any]] = []
        var scriptError: Int = 0
        var routeUsed = "none"
        for candidate in routes {
            scriptError = MessagesAppleScript.execute(candidate.script)
            attempts.append(["route": candidate.name, "apple_event_error": scriptError])
            if scriptError == 0 {
                routeUsed = candidate.name
                break
            }
        }
        let deadline = Date().addingTimeInterval(10)
        var observed: [[String: String]] = []
        var matched: [String: String]?
        repeat {
            observed = try database.rows("SELECT m.ROWID AS row_id, m.text, hex(m.attributedBody) AS body_hex, m.service, m.error, m.is_sent, m.is_delivered, m.is_finished, m.is_from_me, (SELECT COUNT(*) FROM chat_message_join j WHERE j.message_id=m.ROWID) AS chat_joins, (SELECT COUNT(*) FROM chat_message_join j JOIN chat_handle_join peers ON peers.chat_id=j.chat_id JOIN handle h ON h.ROWID=peers.handle_id WHERE j.message_id=m.ROWID AND h.id=?) AS self_joins FROM message m WHERE m.ROWID > ? ORDER BY m.ROWID", bindings: [handle, baseline])
            matched = observed.first { decodedProbeText($0) == text && $0["is_from_me"] == "1" && ($0["self_joins"] ?? "0") != "0" }
            if matched != nil { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        } while Date() < deadline
        let ghosts = observed.filter { ($0["text"] ?? "").isEmpty && $0["service"] == "SMS" && $0["chat_joins"] == "0" && $0["is_from_me"] == "1" }
        var report: [String: Any] = [
            "apple_event_error": scriptError,
            "route": routeUsed,
            "route_attempts": attempts,
            "row_appeared": matched != nil || !ghosts.isEmpty,
            "matching_self_chat_row_appeared": matched != nil,
            "row_attribution": matched != nil ? "matching_self_chat" : ghosts.isEmpty ? "none" : "unattributed_ghost_candidate",
            "text_matched": matched != nil,
            "new_row_count": observed.count,
            "ghost_send_symptoms": !ghosts.isEmpty,
            "ghost_row_count": ghosts.count,
            "payload": PrivateReport.redact(text),
            "disposition": matched != nil ? "locally_observed" : "indeterminate"
        ]
        let fields = ["service", "error", "is_sent", "is_delivered", "is_finished"]
        let candidate = matched ?? ghosts.first
        for field in fields {
            report[field] = candidate?[field] ?? NSNull() as Any
        }
        report["ghost_candidates"] = ghosts.map { row in
            Dictionary(uniqueKeysWithValues: fields.map { ($0, row[$0] ?? "unknown") })
        }
        return report
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

    private static func dumpAccessibility(_ arguments: [String]) throws -> [String: Any] {
        var handle = try selfHandle(arguments)
        if let index = arguments.firstIndex(of: "--approved-contact"), index + 1 < arguments.count {
            guard arguments[index + 1] == "remy" else { throw ProbeFailure.invalidArguments }
            let file = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/apple-bridge/test-contacts.env")
            let lines = try String(contentsOf: file, encoding: .utf8).components(separatedBy: .newlines)
            var contacts: [String: String] = [:]
            for line in lines {
                let parts = line.replacingOccurrences(of: "export ", with: "").split(separator: "=", maxSplits: 1).map(String.init)
                if parts.count == 2 { contacts[parts[0].trimmingCharacters(in: .whitespaces)] = parts[1].trimmingCharacters(in: CharacterSet(charactersIn: " \"'")) }
            }
            handle = contacts["REMY_IMESSAGE_TEST_HANDLE"] ?? contacts["REMY_IMESSAGE_TEST_EMAIL"] ?? ""
            guard !handle.isEmpty else { throw ProbeFailure.invalidArguments }
        }
        guard AXIsProcessTrusted() else { throw ProbeFailure.unavailable }
        if arguments.contains("--compose") {
            let code = MessagesAppleScript.execute("tell application \"Messages\" to activate\n" + MessagesUIScript.keys([45], command: true))
            guard code == 0 else { return ["state": "unavailable", "apple_event_error": code] }
        }
        var components = URLComponents()
        components.scheme = "imessage"
        components.path = "open"
        components.queryItems = [URLQueryItem(name: "address", value: handle)]
        if let index = arguments.firstIndex(of: "--message-guid"), index + 1 < arguments.count {
            let guid = arguments[index + 1]
            let database = try MessagesDatabase()
            let rows = try database.rows("SELECT guid FROM message WHERE guid=? AND (instr(text,'[ab-test ')>0 OR instr(attributedBody,CAST('[ab-test ' AS BLOB))>0)", bindings: [guid])
            guard rows.count == 1 else { throw ProbeFailure.invalidArguments }
            components.queryItems = [URLQueryItem(name: "message-guid", value: guid)]
        }
        guard let url = components.url else { throw ProbeFailure.invalidArguments }
        if !arguments.contains("--current") { try MessagesNavigation.open(url) }
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.1)) }
        guard let app = MessagesNavigation.application,
              LiveMessagesUIDriver().frontmostProcess() == app.processIdentifier else { throw ProbeFailure.unavailable }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.5)
        if let index = arguments.firstIndex(of: "--menu-tag") ?? arguments.firstIndex(of: "--reply-tag"), index + 1 < arguments.count {
            let tag = arguments[index + 1]
            guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]") else { throw ProbeFailure.invalidArguments }
            func find(_ element: AXUIElement) -> [AXUIElement] {
                var identifier: CFTypeRef?
                var description: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXIdentifierAttribute as CFString, &identifier)
                AXUIElementCopyAttributeValue(element, kAXDescriptionAttribute as CFString, &description)
                if identifier as? String == "Sticker", let text = description as? String, text.hasPrefix("Your iMessage,"), text.contains(tag) { return [element] }
                var children: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
                return (children as? [AXUIElement] ?? []).flatMap(find)
            }
            let targets = find(root)
            guard targets.count == 1 else { throw ProbeFailure.unavailable }
            if arguments.contains("--reply-tag") {
                var names: CFArray?
                AXUIElementCopyActionNames(targets[0], &names)
                let reply = (names as? [String] ?? []).filter { $0.hasPrefix("Name:Reply") }
                guard reply.count == 1, AXUIElementPerformAction(targets[0], reply[0] as CFString) == .success else { throw ProbeFailure.unavailable }
                RunLoop.current.run(until: Date().addingTimeInterval(1))
            } else {
            _ = AXUIElementPerformAction(targets[0], "AXScrollToVisible" as CFString)
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
            var position: CFTypeRef?
            var size: CFTypeRef?
            AXUIElementCopyAttributeValue(targets[0], kAXPositionAttribute as CFString, &position)
            AXUIElementCopyAttributeValue(targets[0], kAXSizeAttribute as CFString, &size)
            guard let position, let size else { throw ProbeFailure.unavailable }
            var origin = CGPoint.zero
            var dimensions = CGSize.zero
            AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin)
            AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions)
            let point = CGPoint(x: origin.x + dimensions.width / 2, y: origin.y + dimensions.height / 2)
            let script = MessagesUIScript.controlClick(point, dismissFirst: false)
            let code = MessagesAppleScript.execute(script)
            guard code == 0 else { return ["state": "unavailable", "apple_event_error": code] }
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
            }
        }
        if arguments.contains("--details") {
            var budget = 2000
            func findTitle(_ element: AXUIElement) -> [AXUIElement] {
                guard budget > 0 else { return [] }
                budget -= 1
                var identifier: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXIdentifierAttribute as CFString, &identifier)
                if identifier as? String == "ConversationTitle" { return [element] }
                var children: CFTypeRef?
                AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
                return (children as? [AXUIElement] ?? []).flatMap(findTitle)
            }
            let titles = findTitle(root)
            guard titles.count == 1, AXUIElementPerformAction(titles[0], kAXPressAction as CFString) == .success else { throw ProbeFailure.unavailable }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        var count = 0
        let tree = accessibilityNode(root, depth: 0, count: &count, raw: arguments.contains("--private-values"))
        return ["tree": tree, "node_count": count, "scope": "messages_application_including_transcript", "selection": "self_handle_url_requested", "target_verified": false, "text_redaction": arguments.contains("--private-values") ? "private_unredacted" : "length_sha256", "truncated": count >= 4000]
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
