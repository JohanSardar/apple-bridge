import Foundation
import AppKit
import ApplicationServices
import SourceSupport

extension MessagesParity {
    @UIActor
    static func receiptControl(_ prepared: Prepared) throws -> (MessagesUISession, AXUIElement, Bool) {
        try MessagesAccessibility.openMessage(prepared.nativeState["message_guid"].string, settle: 0.5)
        let session = try MessagesAccessibility.uiSession()
        var state = prepared.nativeState.object
        state["conversation_target"] = .bool(false)
        try MessagesAccessibility.validate(Prepared(nativeState: .object(state), fingerprint: prepared.fingerprint, action: prepared.action))
        let controls = MessagesVariantUI.find(session.root, role: kAXCheckBoxRole, name: "Send Read Receipts")
        if controls.isEmpty {
            let buttons = MessagesVariantUI.find(session.root, role: kAXButtonRole).filter { ["Details", "Show Details", "Conversation Details"].contains(MessagesAccessibility.value($0, kAXDescriptionAttribute)) }
            guard buttons.count == 1 else { throw MessagesUIFailure.menuUnavailable }
            try session.fence()
            try MessagesVariantUI.press(buttons[0])
        }
        let matches = MessagesVariantUI.find(session.root, role: kAXCheckBoxRole, name: "Send Read Receipts")
        guard matches.count == 1 else { throw MessagesUIFailure.targetUnavailable }
        return (session, matches[0], try checkbox(matches[0]))
    }
    @UIActor
    static func checkbox(_ control: AXUIElement) throws -> Bool {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(control, kAXValueAttribute as CFString, &value) == .success,
              let number = value as? NSNumber, [0, 1].contains(number.intValue) else { throw SourceFailure.conflict }
        return number.boolValue
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
            let (session, _, current) = try receiptControl(prepared)
            original = current
            try dismiss(session)
            try receipts(prepared, value: !current)
            try receipts(prepared, value: current)
            let (restoredSession, _, restored) = try receiptControl(prepared)
            try dismiss(restoredSession)
            guard restored == current else { throw SourceFailure.conflict }
            original = nil
            return .object(["operation": .string("messages.set_read_receipts"), "proven": .bool(true), "reason": .string("native_checkbox_and_properties_roundtrip"), "service": prepared.nativeState["service"]])
        } catch {
            if let original { try? receipts(prepared, value: original) }
            return .object(["operation": .string("messages.set_read_receipts"), "proven": .bool(false), "reason": .string("receipt_roundtrip_" + failureReason(error))])
        }
    }
}
