import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesSafetyChecks {
    static func main() async throws {
        let kind = CommandLine.arguments.dropFirst().first ?? ""
        if kind == "media-cleanup" {
            guard let index = CommandLine.arguments.firstIndex(of: "--fixtures"), index + 1 < CommandLine.arguments.count else { throw SourceFailure.invalid }
            let fixture = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[index + 1])))
            print(String(decoding: try JSONEncoder().encode(MessagesMediaCleanupProof.check(fixture)), as: UTF8.self))
            return
        }
        var checks = try await AccessibilityInputChecks.run()
        checks["multipart_capture_repeats_all_parts_in_one_batch"] = try await CommandCaptureChecks.run()
        checks["self_echo_does_not_cancel_same_media_render"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: true, incomingImage: true)
        checks["same_media_backfill_is_reused"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: false, incomingImage: true)
        checks["new_live_image_still_preempts_other_media"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: true, incomingImage: true)
        checks["new_live_media_still_preempts_backfill"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: false, incomingImage: false)
        checks["native_chat_namespace_alias_is_same_direct_address"] = MessagesSelfTestIsolation.nativeChatCandidates("iMessage;-;self@example.com") == ["iMessage;-;self@example.com", "any;-;self@example.com"]
        checks["native_chat_namespace_never_changes_group_or_service"] = MessagesSelfTestIsolation.nativeChatCandidates("SMS;-;self@example.com") == ["SMS;-;self@example.com"] && MessagesSelfTestIsolation.nativeChatCandidates("iMessage;+;group") == ["iMessage;+;group"]
        checks["native_chat_namespace_never_adds_other_address"] = !MessagesSelfTestIsolation.nativeChatCandidates("iMessage;-;self@example.com").contains("any;-;other@example.com")
        checks["already_read_timestamp_is_observed"] = MessagesReadStateFixture.matches(timestamp: 20, latest: 20, unread: false)
        checks["unread_timestamp_is_observed"] = MessagesReadStateFixture.matches(timestamp: 19, latest: 20, unread: true)
        checks["read_state_mismatch_rejected"] = !MessagesReadStateFixture.matches(timestamp: 19, latest: 20, unread: false)
        checks["read_state_mixed_timestamp_units"] = MessagesReadStateFixture.matches(timestamp: 2_000_000_000_000, latest: 2000, unread: false)
        checks["invalid_read_timestamp_rejected"] = !MessagesReadStateFixture.matches(timestamp: .nan, latest: 20, unread: true)
        checks["mark_read_uses_explicit_menu_action"] = MessagesUIActions.readStateAction(unread: false, titles: ["Mark as Read"]) == "Mark as Read"
        checks["mark_unread_uses_explicit_menu_action"] = MessagesUIActions.readStateAction(unread: true, titles: ["Mark as Unread"]) == "Mark as Unread"
        checks["already_read_needs_no_action"] = MessagesUIActions.readStateAction(unread: false, titles: ["Mark as Unread"]) == ""
        checks["already_unread_needs_no_action"] = MessagesUIActions.readStateAction(unread: true, titles: ["Mark as Read"]) == ""
        checks["missing_read_state_menu_rejected"] = MessagesUIActions.readStateAction(unread: false, titles: []) == nil
        checks["ambiguous_read_state_menu_rejected"] = MessagesUIActions.readStateAction(unread: false, titles: ["Mark as Read", "Mark as Unread"]) == nil
        var readinessTime = Date(timeIntervalSince1970: 0)
        var readinessAttempts = 0
        let readyTarget: String = try MessagesUIActions.resolve(until: readinessTime.addingTimeInterval(1), now: { readinessTime }, wait: { readinessTime.addTimeInterval(0.1) }, attempt: {
            readinessAttempts += 1
            if readinessAttempts == 1 { throw MessagesUIFailure.targetUnavailable }
            return "owned"
        })
        checks["transient_empty_tree_is_retried"] = readyTarget == "owned" && readinessAttempts == 2
        var ambiguousAttempts = 0
        do {
            let _: String = try MessagesUIActions.resolve(until: Date().addingTimeInterval(1), wait: {}, attempt: {
                ambiguousAttempts += 1
                throw MessagesUIFailure.targetAmbiguous
            })
            checks["ambiguous_target_is_never_retried"] = false
        } catch { checks["ambiguous_target_is_never_retried"] = error as? MessagesUIFailure == .targetAmbiguous && ambiguousAttempts == 1 }
        do {
            let _: String = try MessagesUIActions.resolve(until: readinessTime, now: { readinessTime }, wait: {}, attempt: { throw MessagesUIFailure.targetUnavailable })
            checks["target_readiness_deadline_is_bounded"] = false
        } catch { checks["target_readiness_deadline_is_bounded"] = error as? MessagesUIFailure == .targetUnavailable }
        let deleteButton = JSON.object(["AXRole": .string("AXButton"), "AXTitle": .string("Delete")])
        let cancelButton = JSON.object(["AXRole": .string("AXButton"), "AXTitle": .string("Cancel")])
        let confirmation = JSON.object(["AXRole": .string("AXSheet"), "children": .array([deleteButton, cancelButton])])
        let appAlert = JSON.object(["AXRole": .string("AXApplication"), "children": .array([confirmation])])
        func deletionButtons(_ root: JSON) -> [JSON] {
            MessagesUIActions.buttons(in: root, title: "Delete", children: { $0["children"].array }, value: { $0[$1].string })
        }
        checks["app_root_confirmation_is_actionable"] = deletionButtons(appAlert) == [deleteButton]
        checks["direct_sheet_confirmation_is_actionable"] = deletionButtons(confirmation) == [deleteButton]
        let nestedAlert = JSON.object(["AXRole": .string("AXWindow"), "children": .array([confirmation])])
        checks["window_sheet_confirmation_is_actionable"] = deletionButtons(nestedAlert) == [deleteButton]
        let menu = JSON.object(["AXRole": .string("AXMenuBar"), "children": .array([deleteButton])])
        checks["unrelated_menu_delete_is_ignored"] = deletionButtons(menu).isEmpty
        let conversationPreferences: [String: Any] = ["EnableReadReceiptForChat": true, "shouldForceToSMS": false, "lastSeenMessageGuid": "earlier", "LSMD": Date(timeIntervalSinceReferenceDate: 1), "numberOfTimesRespondedtoThread": 0]
        var afterSend = conversationPreferences
        afterSend["lastSeenMessageGuid"] = "owned-send"
        afterSend["LSMD"] = Date(timeIntervalSinceReferenceDate: 2)
        afterSend["numberOfTimesRespondedtoThread"] = 1
        checks["owned_send_housekeeping_preserves_preferences"] = try MessagesContentBaseline.preferenceDigest(conversationPreferences) == MessagesContentBaseline.preferenceDigest(afterSend)
        afterSend["EnableReadReceiptForChat"] = false
        checks["receipt_preference_change_detected"] = try MessagesContentBaseline.preferenceDigest(conversationPreferences) != MessagesContentBaseline.preferenceDigest(afterSend)
        afterSend = conversationPreferences
        afterSend["new_preference"] = true
        checks["new_conversation_preference_detected"] = try MessagesContentBaseline.preferenceDigest(conversationPreferences) != MessagesContentBaseline.preferenceDigest(afterSend)
        checks["self_echo_conversation_verified"] = MessagesSelfTestIsolation.matchesConversation(chat: "self", memberships: ["self"], expected: "[ab-test current] fixture", visibleTexts: ["[ab-test current] fixture", "[ab-test current] fixture"])
        checks["wrong_conversation_rejected"] = !MessagesSelfTestIsolation.matchesConversation(chat: "other", memberships: ["self"], expected: "[ab-test current] fixture", visibleTexts: ["[ab-test current] fixture"])
        checks["missing_conversation_text_rejected"] = !MessagesSelfTestIsolation.matchesConversation(chat: "self", memberships: ["self"], expected: "[ab-test current] fixture", visibleTexts: ["different"])
        checks["owned_reaction_identity_allowed"] = MessagesCleanup.canDeleteReaction(rowID: 11, baseline: 10, guid: "reaction", parent: "", owned: ["reaction"], tagged: false)
        checks["tagged_late_reaction_to_owned_parent_allowed"] = MessagesCleanup.canDeleteReaction(rowID: 11, baseline: 10, guid: "reaction", parent: "parent", owned: ["parent"], tagged: true)
        checks["old_reaction_protected"] = !MessagesCleanup.canDeleteReaction(rowID: 10, baseline: 10, guid: "reaction", parent: "parent", owned: ["reaction", "parent"], tagged: true)
        checks["untagged_foreign_reaction_protected"] = !MessagesCleanup.canDeleteReaction(rowID: 11, baseline: 10, guid: "reaction", parent: "parent", owned: ["parent"], tagged: false)
        checks["reaction_to_foreign_parent_protected"] = !MessagesCleanup.canDeleteReaction(rowID: 11, baseline: 10, guid: "reaction", parent: "other", owned: ["parent"], tagged: true)
        checks["owned_unsent_placeholder_allowed"] = MessagesCleanup.canDeleteRetracted(rowID: 11, baseline: 10, guid: "current", owned: ["current"], parts: [.number(0)], text: "")
        checks["existing_unsent_placeholder_protected"] = !MessagesCleanup.canDeleteRetracted(rowID: 10, baseline: 10, guid: "current", owned: ["current"], parts: [.number(0)], text: "")
        checks["foreign_unsent_placeholder_protected"] = !MessagesCleanup.canDeleteRetracted(rowID: 11, baseline: 10, guid: "other", owned: ["current"], parts: [.number(0)], text: "")
        checks["blank_ordinary_message_protected"] = !MessagesCleanup.canDeleteRetracted(rowID: 11, baseline: 10, guid: "current", owned: ["current"], parts: [], text: "")
        let readState = JSON.object(["run_id": .string("current"), "scenario": .string("MSG-READSTATE-REAL"), "baseline_max_row_id": .number(10), "created_chat_key": .string("self")])
        checks["current_read_state_fixture_allowed"] = MessagesReadStateFixture.authorizes(readState, chat: "self", rowID: 11, text: "[ab-test current] fixture")
        checks["older_read_state_target_refused"] = !MessagesReadStateFixture.authorizes(readState, chat: "self", rowID: 10, text: "[ab-test current] fixture")
        checks["other_run_read_state_target_refused"] = !MessagesReadStateFixture.authorizes(readState, chat: "self", rowID: 11, text: "[ab-test other] fixture")
        checks["other_read_state_chat_refused"] = !MessagesReadStateFixture.authorizes(readState, chat: "other", rowID: 11, text: "[ab-test current] fixture")
        checks["untagged_read_state_target_refused"] = !MessagesReadStateFixture.authorizes(readState, chat: "self", rowID: 11, text: "existing message")
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
    }
}
