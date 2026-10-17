import Foundation
import Decoders
import Contacts
import AppKit
import ApplicationServices

public enum MessagesProbes {
    public static func discoverSelfHandles() throws -> (handles: [String], source: String) {
        if CNContactStore.authorizationStatus(for: .contacts) == .authorized {
            let store = CNContactStore()
            if let contact = try? store.unifiedMeContactWithKeys(toFetch: [CNContactPhoneNumbersKey as CNKeyDescriptor, CNContactEmailAddressesKey as CNKeyDescriptor]) {
                let handles = contact.phoneNumbers.map { $0.value.stringValue } + contact.emailAddresses.map { $0.value as String }
                let normalized = Array(Set(handles.map(normalize).filter { !$0.isEmpty })).sorted()
                if !normalized.isEmpty { return (normalized, "contacts_me") }
            }
        }
        let database = try MessagesDatabase()
        let columns = Set(try database.rows("PRAGMA table_info(message)").compactMap { $0["name"] })
        let available = ["destination_caller_id", "account_login"].filter { columns.contains($0) }
        guard !available.isEmpty else { throw ProbeFailure.unavailable }
        var handles: Set<String> = []
        for column in available {
            for row in try database.rows("SELECT DISTINCT \(column) AS handle FROM message WHERE is_from_me = 1 AND \(column) IS NOT NULL") {
                if let raw = row["handle"] {
                    let value = normalize(raw)
                    if !value.isEmpty { handles.insert(value) }
                }
            }
        }
        guard !handles.isEmpty else { throw ProbeFailure.noSelfHandle }
        return (handles.sorted(), "messages_own_account")
    }

    public static func normalize(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["iMessage:", "SMS:", "RCS:", "mailto:", "tel:", "P:", "E:"] {
            if text.hasPrefix(prefix) { text.removeFirst(prefix.count) }
        }
        if text.contains("@") { return text.lowercased() }
        return text.filter { $0.isNumber || $0 == "+" }
    }

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
        case "self-handles":
            let result = try discoverSelfHandles()
            let report: [String: Any] = ["handles": result.handles, "source": result.source, "index_base": 0]
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
        let handle = try selfHandle(arguments)
        guard AXIsProcessTrusted() else { throw ProbeFailure.unavailable }
        guard let url = URL(string: "imessage://" + handle.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed)!) else { throw ProbeFailure.invalidArguments }
        guard NSWorkspace.shared.open(url) else { throw ProbeFailure.unavailable }
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.1)) }
        guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").first,
              NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw ProbeFailure.unavailable }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.5)
        var count = 0
        let tree = accessibilityNode(root, depth: 0, count: &count)
        return ["tree": tree, "node_count": count, "scope": "messages_application_including_transcript", "selection": "self_handle_url_requested", "target_verified": false, "text_redaction": "length_sha256", "truncated": count >= 4000]
    }

    private static func accessibilityNode(_ element: AXUIElement, depth: Int, count: inout Int) -> [String: Any] {
        count += 1
        var node: [String: Any] = [:]
        for attribute in [kAXRoleAttribute, kAXSubroleAttribute, kAXIdentifierAttribute, kAXDescriptionAttribute, kAXTitleAttribute, kAXValueAttribute, kAXHelpAttribute] {
            var value: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success, let text = value as? String {
                node[attribute] = attribute == kAXRoleAttribute || attribute == kAXSubroleAttribute || attribute == kAXIdentifierAttribute ? text as Any : PrivateReport.redact(text) as Any
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
                    result.append(accessibilityNode(child, depth: depth + 1, count: &count))
                }
                node["children"] = result
            }
        }
        return node
    }
}
