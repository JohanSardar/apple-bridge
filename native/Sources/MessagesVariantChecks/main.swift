import Foundation
import MessagesSource
import SourceSupport

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
