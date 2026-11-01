import Foundation
import MessagesSource
import SourceSupport

private final class ForwardedNativeFlagReceiver: NSObject {
    @objc func readyForNativeFlag() -> Bool { true }
}
private final class ForwardedNativeFlagProxy: NSObject {
    private let receiver = ForwardedNativeFlagReceiver()
    override func responds(to selector: Selector!) -> Bool { receiver.responds(to: selector) || super.responds(to: selector) }
    override func forwardingTarget(for selector: Selector!) -> Any? { receiver.responds(to: selector) ? receiver : super.forwardingTarget(for: selector) }
}

@main
struct MessagesVariantChecks {
    static func main() async throws {
        let kind = CommandLine.arguments.dropFirst().first ?? ""
        if kind == "media-cleanup" {
            guard let index = CommandLine.arguments.firstIndex(of: "--fixtures"), index + 1 < CommandLine.arguments.count else { throw SourceFailure.invalid }
            let fixture = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[index + 1])))
            print(String(decoding: try JSONEncoder().encode(MessagesMediaCleanupProof.check(fixture)), as: UTF8.self))
            return
        }
        let range = JSON.object(["utf16_location": .number(0), "utf16_length": .number(4), "styles": .array([.string("bold")])])
        let variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([range])])
        let args = JSON.object(["text": .string("test"), "variant": variant])
        let data: [String: JSON] = ["text": .string("test"), "is_from_me": .bool(true), "error_code": .number(0), "effect": .string("com.apple.messages.effect.loud"), "formatting": .array([range])]
        var checks = try await AccessibilityInputChecks.run()
        let address = try MessagesNavigation.addressURL("self+test@example.com")
        checks["self_chat_navigation_uses_exact_address"] = URLComponents(url: address, resolvingAgainstBaseURL: false)?.queryItems == [URLQueryItem(name: "address", value: "self+test@example.com")]
        let deepLink = try MessagesNavigation.messageURL("test-guid&other=value")
        let deepLinkComponents = URLComponents(url: deepLink, resolvingAgainstBaseURL: false)
        checks["message_deep_link_uses_open_path"] = deepLinkComponents?.scheme == "imessage" && deepLinkComponents?.host == nil && deepLinkComponents?.path == "open"
        checks["message_deep_link_preserves_exact_guid"] = deepLinkComponents?.queryItems == [URLQueryItem(name: "message-guid", value: "test-guid&other=value")]
        checks["multipart_capture_repeats_all_parts_in_one_batch"] = try await CommandCaptureChecks.run()
        checks["self_echo_does_not_cancel_same_media_render"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: true, incomingImage: true)
        checks["same_media_backfill_is_reused"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: false, incomingImage: true)
        checks["new_live_image_still_preempts_other_media"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: true, incomingImage: true)
        checks["new_live_media_still_preempts_backfill"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: false, incomingImage: false)
        checks["native_chat_namespace_alias_is_same_direct_address"] = MessagesSelfTestIsolation.nativeChatCandidates("iMessage;-;self@example.com") == ["iMessage;-;self@example.com", "any;-;self@example.com"]
        checks["native_chat_namespace_never_changes_group_or_service"] = MessagesSelfTestIsolation.nativeChatCandidates("SMS;-;self@example.com") == ["SMS;-;self@example.com"] && MessagesSelfTestIsolation.nativeChatCandidates("iMessage;+;group") == ["iMessage;+;group"]
        checks["native_chat_namespace_never_adds_other_address"] = !MessagesSelfTestIsolation.nativeChatCandidates("iMessage;-;self@example.com").contains("any;-;other@example.com")
        checks["forwarded_native_receiver_supported"] = await MainActor.run { MessagesSendVariants.syntheticForwardedNativeFlag(ForwardedNativeFlagProxy(), selector: "readyForNativeFlag") }
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
        } else if kind == "schedule" {
            let date = timestamp(Date(timeIntervalSince1970: 2_000_000_000))
            let args = JSON.object(["text": .string("test"), "variant": .object(["kind": .string(kind), "scheduled_for": .string(date)])])
            var sent = data
            sent["schedule"] = .object(["state": .string("native_1"), "scheduled_for": .string(date)])
            checks["scheduled_date_exact_readback"] = MessagesSendVariants.matches(args, data: .object(sent))
            sent["schedule"] = .object(["state": .string("native_1"), "scheduled_for": .string(timestamp(Date(timeIntervalSince1970: 2_000_000_060)))])
            checks["wrong_scheduled_date_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
            checks["plain_send_rejected"] = !MessagesSendVariants.matches(args, data: .object(data))
        } else if kind == "mention" {
            let args = JSON.object(["text": .string("test @self@example.com"), "variant": .object(["kind": .string(kind), "handle_id": .string("self@example.com"), "utf16_location": .number(5), "utf16_length": .number(17)])])
            var sent = data
            sent["text"] = .string("test Self")
            sent["parts"] = .array([.object(["mention": .string("self@example.com"), "utf16_location": .number(5), "utf16_length": .number(4)])])
            checks["mention_identity_readback"] = MessagesSendVariants.matches(args, data: .object(sent))
            sent["text"] = .string("test Self unexpected")
            checks["extra_mention_text_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
            sent["text"] = .string("test Self")
            sent["parts"] = .array([.object(["mention": .string("other@example.com"), "utf16_location": .number(5), "utf16_length": .number(4)])])
            checks["other_mention_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
            checks["plain_send_rejected"] = !MessagesSendVariants.matches(args, data: .object(data))
        } else if kind == "audio" {
            checks["native_audio_factory_sets_flag"] = MessagesSendVariants.syntheticNativeAudioFlag()
            let args = JSON.object(["text": .string(""), "files": .array([.object(["name": .string("synthetic.caf")])]), "variant": .object(["kind": .string(kind)])])
            var sent = data
            sent["text"] = .string("")
            sent["audio"] = .object(["is_audio_message": .bool(true)])
            checks["native_audio_flag_required"] = MessagesSendVariants.matches(args, data: .object(sent))
            checks["late_echo_does_not_hide_deletion"] = MessagesCleanup.removedOwnedIDs(before: ["owned-a", "owned-b"], after: ["owned-b", "owned-echo"]) == ["owned-a"]
            checks["new_echo_alone_is_not_deletion"] = MessagesCleanup.removedOwnedIDs(before: ["owned-a"], after: ["owned-a", "owned-echo"]).isEmpty
            checks["earlier_draft_preserved"] = !MessagesUISession.canReplaceDraft("[ab-test old] draft", with: "[ab-test current] reply")
            checks["personal_draft_preserved"] = !MessagesUISession.canReplaceDraft("personal draft", with: "[ab-test current] reply")
            checks["current_draft_replace_allowed"] = MessagesUISession.canReplaceDraft("[ab-test current] draft", with: "[ab-test current] reply")
            checks["verified_existing_fixture_chat_allowed"] = MessagesSelfTestIsolation.qualifiesFixtureChat("self", rowID: 5, baseline: 10, existing: "self")
            checks["different_existing_fixture_chat_refused"] = !MessagesSelfTestIsolation.qualifiesFixtureChat("other", rowID: 5, baseline: 10, existing: "self")
            checks["unregistered_existing_fixture_chat_refused"] = !MessagesSelfTestIsolation.qualifiesFixtureChat("self", rowID: 5, baseline: 10, existing: "")
            checks["new_fixture_chat_allowed"] = MessagesSelfTestIsolation.qualifiesFixtureChat("new", rowID: 11, baseline: 10, existing: "")
            checks["empty_fixture_chat_refused"] = !MessagesSelfTestIsolation.qualifiesFixtureChat("", rowID: 11, baseline: 10, existing: "")
            checks["scheduled_send_does_not_require_early_delivery"] = !MessagesVariantProof.requiresSelfEcho("schedule") && MessagesVariantProof.requiresSelfEcho("mention") && MessagesVariantProof.requiresSelfEcho("audio")
            checks["date_commit_failure_retains_cleanup_obligation"] = MessagesSendVariants.dispatchFailure(SendVariantFailure.draftChanged, sendStarted: true) as? SendVariantFailure == .sendReceiptUnknown
            checks["pre_send_failure_remains_rejected"] = MessagesSendVariants.dispatchFailure(SendVariantFailure.draftPresent, sendStarted: false) as? SendVariantFailure == .draftPresent
            do {
                _ = try await MessagesCleanup.run(["--owned-only"])
                checks["unscoped_owned_cleanup_refused_before_native_access"] = false
            } catch ProbeFailure.invalidArguments {
                checks["unscoped_owned_cleanup_refused_before_native_access"] = true
            } catch {
                checks["unscoped_owned_cleanup_refused_before_native_access"] = false
            }
            checks["late_tagged_echo_owned"] = MessagesCleanup.allowsLateEcho(rowID: 11, baseline: 10, text: "[ab-test run_current] fixture", tag: "[ab-test run_current]")
            checks["earlier_residue_refused"] = !MessagesCleanup.allowsLateEcho(rowID: 9, baseline: 10, text: "[ab-test run_current] fixture", tag: "[ab-test run_current]")
            checks["other_run_echo_refused"] = !MessagesCleanup.allowsLateEcho(rowID: 11, baseline: 10, text: "[ab-test run_earlier] fixture", tag: "[ab-test run_current]")
            sent["audio"] = .object(["is_audio_message": .bool(false)])
            checks["ordinary_attachment_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
            sent["audio"] = .object(["is_audio_message": .bool(true)])
            sent["is_from_me"] = .bool(false)
            checks["incoming_audio_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
        } else {
            checks["unqualified_variant_rejected"] = !MessagesSendVariants.matches(args, data: .object(data))
        }
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
        if checks.values.contains(false) { exit(1) }
    }
}
