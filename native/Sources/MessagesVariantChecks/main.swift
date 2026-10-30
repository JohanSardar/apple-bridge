import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesVariantChecks {
    static func main() throws {
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
        var checks = [String: Bool]()
        if ["formatting", "effect"].contains(kind) {
            checks["matching_sent_attributes"] = MessagesSendVariants.matches(args, data: .object(data))
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
            sent["parts"] = .array([.object(["mention": .string("other@example.com"), "utf16_location": .number(5), "utf16_length": .number(4)])])
            checks["other_mention_rejected"] = !MessagesSendVariants.matches(args, data: .object(sent))
            checks["plain_send_rejected"] = !MessagesSendVariants.matches(args, data: .object(data))
        } else if kind == "audio" {
            let args = JSON.object(["text": .string(""), "files": .array([.object(["name": .string("synthetic.caf")])]), "variant": .object(["kind": .string(kind)])])
            var sent = data
            sent["text"] = .string("")
            sent["audio"] = .object(["is_audio_message": .bool(true)])
            checks["native_audio_flag_required"] = MessagesSendVariants.matches(args, data: .object(sent))
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
