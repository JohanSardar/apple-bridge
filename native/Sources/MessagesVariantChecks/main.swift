import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesVariantChecks {
    static func main() async throws {
        let kind = CommandLine.arguments.dropFirst().first ?? ""
        let range = JSON.object(["utf16_location": .number(0), "utf16_length": .number(4), "styles": .array([.string("bold")])])
        let variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([range])])
        let args = JSON.object(["text": .string("test"), "variant": variant])
        let data: [String: JSON] = ["text": .string("test"), "is_from_me": .bool(true), "error_code": .number(0), "effect": .string("com.apple.messages.effect.loud"), "formatting": .array([range])]
        var checks = try await AccessibilityInputChecks.run()
        checks["unchecked_alert_menu_is_unmuted"] = MessagesParity.alertMenuTitles(["Hide Alerts"], checked: []) == ["Hide Alerts"]
        checks["checked_alert_menu_is_muted"] = MessagesParity.alertMenuTitles(["Hide Alerts", "Pin"], checked: ["Hide Alerts"]) == ["Show Alerts", "Pin"]
        checks["other_checked_menu_items_preserved"] = MessagesParity.alertMenuTitles(["Hide Alerts", "Pin"], checked: ["Pin"]) == ["Hide Alerts", "Pin"]
        let proof = JSON.object(["os_build": .string("synthetic-os"), "version": .number(1), "complete": .bool(true), "helper_digest": .string("synthetic-helper")])
        checks["current_build_qualified"] = MessagesSendVariants.validProof(proof, osBuild: "synthetic-os", helperDigest: "synthetic-helper")
        checks["stale_build_rejected"] = !MessagesSendVariants.validProof(proof, osBuild: "synthetic-os", helperDigest: "other-helper")
        checks["stale_os_rejected"] = !MessagesSendVariants.validProof(proof, osBuild: "other-os", helperDigest: "synthetic-helper")
        checks["missing_build_rejected"] = !MessagesSendVariants.validProof(proof, osBuild: "synthetic-os", helperDigest: "")
        checks["exact_recipient_email"] = MessagesParityFixture.recipientMatches("Self <SELF@example.com>", handle: "self@example.com")
        checks["other_recipient_email_rejected"] = !MessagesParityFixture.recipientMatches("other-self@example.com", handle: "self@example.com")
        checks["additional_recipient_email_rejected"] = !MessagesParityFixture.recipientMatches("self@example.com, other@example.com", handle: "self@example.com")
        checks["exact_recipient_phone"] = MessagesParityFixture.recipientMatches("Self (+1 202-555-0123)", handle: "+12025550123")
        checks["phone_substring_rejected"] = !MessagesParityFixture.recipientMatches("+1 202-555-0123", handle: "2025550123")
        checks["existing_self_chat_accepted"] = MessagesSelfTestIsolation.isSelfChat(["iMessage:E:SELF%40example.com"], own: ["self@example.com"])
        checks["other_chat_rejected"] = !MessagesSelfTestIsolation.isSelfChat(["other@example.com"], own: ["self@example.com"])
        checks["group_with_self_rejected"] = !MessagesSelfTestIsolation.isSelfChat(["self@example.com", "other@example.com"], own: ["self@example.com"])
        checks["empty_chat_rejected"] = !MessagesSelfTestIsolation.isSelfChat([], own: ["self@example.com"])
        checks["phone_suffix_is_not_self"] = !MessagesSelfTestIsolation.isSelfChat(["2025550123"], own: ["+12025550123"])
        checks["exact_phone_is_self"] = MessagesSelfTestIsolation.isSelfChat(["tel:+1 (202) 555-0123"], own: ["+12025550123"])
        checks["owned_deleted_selection_allowed"] = MessagesSelfTestIsolation.ownsDeletedSelection(["new-echo", "new-send"], created: ["new-send", "new-echo"])
        checks["existing_deleted_selection_rejected"] = !MessagesSelfTestIsolation.ownsDeletedSelection(["old", "new-send"], created: ["new-send"])
        checks["incomplete_deleted_selection_rejected"] = !MessagesSelfTestIsolation.ownsDeletedSelection(["new-send"], created: ["new-send", "new-echo"])
        checks["empty_deleted_selection_rejected"] = !MessagesSelfTestIsolation.ownsDeletedSelection([], created: [])
        checks["duplicate_deleted_selection_rejected"] = !MessagesSelfTestIsolation.ownsDeletedSelection(["new-send", "new-send"], created: ["new-send"])
        checks["run_message_owned"] = MessagesSelfTestIsolation.ownsMessage(rowID: 101, text: "[ab-test synthetic] new", tag: "[ab-test synthetic]", baseline: 100)
        checks["baseline_message_protected"] = !MessagesSelfTestIsolation.ownsMessage(rowID: 100, text: "[ab-test synthetic] old", tag: "[ab-test synthetic]", baseline: 100)
        checks["other_run_protected"] = !MessagesSelfTestIsolation.ownsMessage(rowID: 101, text: "[ab-test other] new", tag: "[ab-test synthetic]", baseline: 100)
        checks["untagged_message_protected"] = !MessagesSelfTestIsolation.ownsMessage(rowID: 101, text: "new", tag: "[ab-test synthetic]", baseline: 100)
        checks["quoted_tag_protected"] = !MessagesSelfTestIsolation.ownsMessage(rowID: 101, text: "quoted [ab-test synthetic] new", tag: "[ab-test synthetic]", baseline: 100)
        checks["invalid_tag_rejected"] = !MessagesSelfTestIsolation.ownsMessage(rowID: 101, text: " new", tag: "", baseline: 100)
        let unreadBaseline = MessagesSelfTestReadState(unreadIDs: ["old-a", "old-b"], unreadCount: 2)
        checks["unread_exact_restore"] = unreadBaseline.matches(unreadBaseline)
        checks["unread_marker_requires_native_restore"] = unreadBaseline.needsUnreadRestore(MessagesSelfTestReadState(unreadIDs: ["old-a", "old-b"], unreadCount: 0))
        checks["unread_identity_restore_is_idempotent"] = !unreadBaseline.needsUnreadRestore(MessagesSelfTestReadState(unreadIDs: ["old-a", "old-b"], unreadCount: 1))
        checks["unread_unchanged_needs_no_action"] = !unreadBaseline.needsUnreadRestore(unreadBaseline)
        checks["unread_exact_identities_restore"] = unreadBaseline.matches(MessagesSelfTestReadState(unreadIDs: ["old-b", "old-a"], unreadCount: 1))
        checks["unread_count_restore"] = unreadBaseline.matches(MessagesSelfTestReadState(unreadIDs: ["old-c"], unreadCount: 2))
        checks["unread_count_mismatch_rejected"] = !unreadBaseline.matches(MessagesSelfTestReadState(unreadIDs: [], unreadCount: 0))
        checks["unread_read_flags_mismatch_rejected"] = !MessagesSelfTestReadState(unreadIDs: [], unreadCount: 0).matches(MessagesSelfTestReadState(unreadIDs: ["old-a"], unreadCount: 0))
        let resumedUnread = unreadBaseline.appendingNewer(MessagesSelfTestReadState(unreadIDs: ["new-a"], unreadCount: 1))
        checks["unread_resume_preserves_new_messages"] = resumedUnread.unreadIDs == ["old-a", "old-b", "new-a"] && resumedUnread.unreadCount == 3
        let unchangedUnread = unreadBaseline.appendingNewer(MessagesSelfTestReadState(unreadIDs: [], unreadCount: 0))
        checks["unread_resume_without_new_messages_is_idempotent"] = unchangedUnread.unreadIDs == unreadBaseline.unreadIDs && unchangedUnread.unreadCount == unreadBaseline.unreadCount
        let pins = try PropertyListSerialization.data(fromPropertyList: ["pD": ["pP": ["synthetic-a", "synthetic-b"]]], format: .binary, options: 0)
        checks["native_pin_order"] = try MessagesPins.identifiers(pins) == ["synthetic-a", "synthetic-b"]
        checks["valid_pin_indices"] = try [-1, 0, 8].allSatisfy { try MessagesPins.requestedIndex(.number(Double($0))) == $0 }
        for value: JSON in [.null, .string("0"), .bool(false), .number(0.5), .number(-2), .number(9)] {
            do { _ = try MessagesPins.requestedIndex(value); checks["invalid_pin_index_rejected"] = false }
            catch { if checks["invalid_pin_index_rejected"] == nil { checks["invalid_pin_index_rejected"] = true } }
        }
        for value: [String: Any] in [["pD": ["pP": ["same", "same"]]], ["pD": ["pP": [""]]], ["pD": ["pP": [1]]], [:]] {
            let invalid = try PropertyListSerialization.data(fromPropertyList: value, format: .binary, options: 0)
            do { _ = try MessagesPins.identifiers(invalid); checks["invalid_pin_state_rejected"] = false }
            catch { if checks["invalid_pin_state_rejected"] == nil { checks["invalid_pin_state_rejected"] = true } }
        }
        if ["formatting", "effect"].contains(kind) {
            checks["matching_sent_attributes"] = MessagesSendVariants.matches(args, data: .object(data))
            var echo = data
            echo["is_from_me"] = .bool(false)
            checks["matching_echo_attributes"] = MessagesSendVariants.matchesEcho(args, data: .object(echo))
            echo[kind == "effect" ? "effect" : "formatting"] = kind == "effect" ? .string("notloud") : .array([])
            checks["plain_echo_rejected"] = !MessagesSendVariants.matchesEcho(args, data: .object(echo))
            checks["outgoing_is_not_echo"] = !MessagesSendVariants.matchesEcho(args, data: .object(data))
            var wrong = data
            wrong[kind == "effect" ? "effect" : "formatting"] = kind == "effect" ? .string("notloud") : .array([])
            checks["plain_send_rejected"] = !MessagesSendVariants.matches(args, data: .object(wrong))
            for (field, value) in [("is_from_me", JSON.bool(false)), ("text", .string("other")), ("error_code", .number(1))] {
                var changed = data
                changed[field] = value
                checks[field + "_mismatch_rejected"] = !MessagesSendVariants.matches(args, data: .object(changed))
            }
        } else {
            checks["unqualified_variant_rejected"] = !MessagesSendVariants.matches(args, data: .object(data))
        }
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
        if checks.values.contains(false) { exit(1) }
    }
}
