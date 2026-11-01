import Foundation
import AppKit
import ApplicationServices
import SourceSupport

extension MessagesParity {
    public static func checkboxValue(_ raw: JSON) -> Bool? {
        switch raw {
        case .number(0), .string("0"): return false
        case .number(1), .string("1"): return true
        default: return nil
        }
    }
    public static func detailsControlIndex(_ controls: [JSON]) -> Int? {
        let titles = controls.indices.filter { controls[$0]["identifier"].string == "ConversationTitle" }
        if !titles.isEmpty { return titles.count == 1 ? titles[0] : nil }
        let matches = controls.indices.filter { controls[$0]["role"].string == kAXButtonRole && ["Details", "Show Details", "Conversation Details"].contains(controls[$0]["description"].string) }
        return matches.count == 1 ? matches[0] : nil
    }
    @UIActor
    static func receiptControl(_ prepared: Prepared) throws -> (MessagesUISession, AXUIElement, Bool) {
        try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string, settle: 0.5, resetEditor: false)
        let session = try MessagesAccessibility.uiSession()
        var returned = false
        var report: [String: Any] = ["passed": false]
        defer {
            if !returned { try? dismiss(session) }
            try? PrivateReport.write(report, to: PrivateReport.directory.appendingPathComponent("messages-receipt-controls.json"))
        }
        try MessagesAccessibility.validateConversation(prepared)
        let controls = MessagesVariantUI.find(session.root, role: kAXCheckBoxRole, name: "Send Read Receipts")
        report["initial_receipt_control_count"] = controls.count
        if controls.isEmpty {
            let nodes = MessagesVariantUI.find(session.root)
            let descriptors = nodes.map { JSON.object(["identifier": .string(MessagesAccessibility.value($0, kAXIdentifierAttribute)), "role": .string(MessagesAccessibility.value($0, kAXRoleAttribute)), "description": .string(MessagesAccessibility.value($0, kAXDescriptionAttribute))]) }
            report["conversation_title_count"] = descriptors.filter { $0["identifier"].string == "ConversationTitle" }.count
            guard let index = detailsControlIndex(descriptors) else { throw MessagesUIFailure.menuUnavailable }
            try session.fence()
            try MessagesVariantUI.press(nodes[index])
        }
        var matches = MessagesVariantUI.find(session.root, role: kAXCheckBoxRole, name: "Send Read Receipts")
        let deadline = Date().addingTimeInterval(3)
        while matches.isEmpty, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            matches = MessagesVariantUI.find(session.root, role: kAXCheckBoxRole, name: "Send Read Receipts")
        }
        report["receipt_control_count"] = matches.count
        guard matches.count == 1 else { throw MessagesUIFailure.targetUnavailable }
        let checked = try checkbox(matches[0])
        report["passed"] = true
        returned = true
        return (session, matches[0], checked)
    }
    @UIActor
    static func checkbox(_ control: AXUIElement) throws -> Bool {
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(control, kAXValueAttribute as CFString, &value)
        let raw: JSON
        if let number = value as? NSNumber { raw = .number(number.doubleValue) }
        else if let text = value as? String { raw = .string(text) }
        else { raw = .null }
        let checked = checkboxValue(raw)
        try? PrivateReport.write(["value_read": status == .success, "numeric_value": value is NSNumber, "string_value": value is String, "recognized": checked != nil, "value_is_minus_one": raw == .number(-1) || raw == .string("-1"), "value_is_two": raw == .number(2) || raw == .string("2")], to: PrivateReport.directory.appendingPathComponent("messages-receipt-value-controls.json"))
        guard status == .success, let checked else { throw SourceFailure.conflict }
        return checked
    }
    @UIActor
    static func receipts(_ prepared: Prepared, value: Bool) throws {
        let (session, control, current) = try receiptControl(prepared)
        defer { try? dismiss(session) }
        if current != value {
            try session.fence()
            try MessagesVariantUI.press(control)
        }
        guard try checkbox(control) == value else { throw SourceFailure.conflict }
        let deadline = Date().addingTimeInterval(3)
        repeat {
            if (try properties(prepared.action["native_key"].string)["EnableReadReceiptForChat"] as? NSNumber)?.boolValue == value { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw SourceFailure.conflict
    }
    @UIActor
    static func proveReceipts(_ prepared: Prepared) -> JSON {
        var original: Bool?
        do {
            guard try !prepared.nativeState["existing_self_chat"].bool || MessagesSelfTestBaseline.receiptSetting(prepared.action["native_key"].string) != "default" else {
                return .object(["operation": .string("messages.set_read_receipts"), "proven": .bool(false), "restored": .bool(true), "reason": .string("Read receipts inherit the account setting. The Messages checkbox cannot restore inheritance after a test toggle, so the existing setting was preserved."), "service": prepared.nativeState["service"]])
            }
            let (session, _, current) = try receiptControl(prepared)
            original = current
            try dismiss(session)
            try receipts(prepared, value: !current)
            try receipts(prepared, value: current)
            let (restoredSession, _, restored) = try receiptControl(prepared)
            try dismiss(restoredSession)
            guard restored == current else { throw SourceFailure.conflict }
            original = nil
            return .object(["operation": .string("messages.set_read_receipts"), "proven": .bool(true), "restored": .bool(true), "reason": .string("native_checkbox_and_properties_roundtrip"), "service": prepared.nativeState["service"]])
        } catch {
            var restored = original == nil
            if let original {
                try? receipts(prepared, value: original)
                if let (session, _, current) = try? receiptControl(prepared) { restored = current == original; try? dismiss(session) }
            }
            return .object(["operation": .string("messages.set_read_receipts"), "proven": .bool(false), "restored": .bool(restored), "reason": .string("receipt_roundtrip_" + failureReason(error))])
        }
    }
}
