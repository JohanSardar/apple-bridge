import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesVariantChecks {
    static func main() throws {
        let kind = CommandLine.arguments.dropFirst().first ?? ""
        let range = JSON.object(["utf16_location": .number(0), "utf16_length": .number(4), "styles": .array([.string("bold")])])
        let variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([range])])
        let args = JSON.object(["text": .string("test"), "variant": variant])
        let data: [String: JSON] = ["text": .string("test"), "is_from_me": .bool(true), "error_code": .number(0), "effect": .string("com.apple.messages.effect.loud"), "formatting": .array([range])]
        var checks = [String: Bool]()
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
