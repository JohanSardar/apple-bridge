import Foundation
import SourceSupport

public extension RemindersStoreSnapshot {
    func reminderFields(_ row: [String: JSON], parentKeys: [Int: String]) -> JSON {
        let parent = parentKeys[Int(row["ZPARENTREMINDER"]?.number ?? 0)] ?? ""
        var fields: [String: JSON] = [
            "store_identifier": row["ZIDENTIFIER"] ?? .null,
            "flagged": Self.boolean(row["ZFLAGGED"]),
            "parent_id": parent.isEmpty ? .null : Self.reference("reminders.reminder", parent),
            "created_at": Self.date(row["ZCREATIONDATE"]),
            "ics_display_order": row["ZICSDISPLAYORDER"] ?? .null,
            "store_status": .string("partial")
        ]
        fields["store_coverage"] = .object([
            "flagged": Self.coverage(Self.boolean(row["ZFLAGGED"]) == .null ? "unsupported" : "complete"),
            "parent_id": Self.coverage(row["ZPARENTREMINDER"] == .null || row["ZPARENTREMINDER"]?.number == 0 || !parent.isEmpty ? "complete" : "partial"),
            "store_identifier": Self.coverage(row["ZIDENTIFIER"] == nil ? "unsupported" : "complete"),
            "created_at": Self.coverage(fields["created_at"] == .null ? "unavailable" : "complete"),
            "ics_display_order": Self.coverage(row["ZICSDISPLAYORDER"] == nil ? "unsupported" : "complete"),
            "manual_order": Self.coverage("unsupported", reason: "mergeable_ordering_decoder_unverified"),
            "early_reminders": Self.coverage("unsupported", reason: "due_date_delta_decoder_unverified"),
            "section_id": Self.coverage("unsupported", reason: "section_membership_decoder_unverified")
        ])
        return .object(fields)
    }

    func listFields(_ row: [String: JSON], listKeys: [Int: String]) -> JSON {
        var fields: [String: JSON] = ["store_identifier": row["ZIDENTIFIER"] ?? .null, "store_status": .string("partial")]
        var coverage: [String: JSON] = ["store_identifier": Self.coverage(row["ZIDENTIFIER"] == nil ? "unsupported" : "complete")]
        for (column, field) in [
            ("ZBADGEEMBLEM", "badge_emblem"), ("ZCOLOR", "store_color"),
            ("ZDADISPLAYORDER", "display_order"), ("ZSORTINGSTYLE", "sorting_style"),
            ("ZGROCERYLOCALEID", "grocery_locale"), ("ZSHARINGSTATUS", "sharing_status"),
            ("ZSHAREDOWNERNAME", "shared_owner_name"), ("ZSMARTLISTTYPE", "smart_list_type")
        ] {
            fields[field] = row[column] ?? .null
            coverage[field] = Self.coverage(row[column] == nil ? "unsupported" : "complete")
        }
        for (column, field) in [
            ("ZISGROUP", "is_group"), ("ZISPINNEDBYCURRENTUSER", "pinned"),
            ("ZSHOULDCATEGORIZEGROCERYITEMS", "categorize_groceries")
        ] {
            fields[field] = Self.boolean(row[column])
            coverage[field] = Self.coverage(fields[field] == .null ? "unavailable" : "complete")
        }
        fields["pinned_at"] = Self.date(row["ZPINNEDDATE"])
        coverage["pinned_at"] = Self.coverage(row["ZPINNEDDATE"] == nil ? "unsupported" : "complete")
        coverage["store_color"] = Self.coverage("unsupported", reason: "color_archive_unverified")
        let candidates = Set(["ZPARENTLIST", "ZPARENTLIST1"].compactMap { row[$0] }.filter { $0 != .null && $0.number > 0 }.map { Int($0.number) })
        if candidates.count == 1, let key = candidates.first, let parent = listKeys[key], !parent.isEmpty {
            fields["parent_id"] = Self.reference("reminders.list", parent)
        } else { fields["parent_id"] = .null }
        coverage.merge([
            "parent_id": Self.coverage(candidates.count > 1 ? "unsupported" : !candidates.isEmpty && fields["parent_id"] == .null ? "partial" : "complete"),
            "sections": Self.coverage("unsupported", reason: "section_order_decoder_unverified"),
            "smart_filter": Self.coverage("unsupported", reason: "smart_filter_decoder_unverified")
        ]) { _, new in new }
        fields["store_coverage"] = .object(coverage)
        return .object(fields)
    }

    static func boolean(_ value: JSON?) -> JSON {
        guard let value, value != .null else { return .null }
        switch value {
        case .integer(let number) where number <= 1: return .bool(number == 1)
        case .number(let number) where number == 0 || number == 1: return .bool(number == 1)
        default: return .null
        }
    }

    static func date(_ value: JSON?) -> JSON {
        guard let value else { return .null }
        switch value {
        case .integer, .number: break
        default: return .null
        }
        guard value.number.isFinite else { return .null }
        return .string(timestamp(Date(timeIntervalSinceReferenceDate: value.number)))
    }

    static func reference(_ collection: String, _ key: String) -> JSON {
        .object(["$ref": .object(["collection": .string(collection), "native_key": .string(key)])])
    }

    static func coverage(_ status: String, reason: String = "") -> JSON {
        var fields: [String: JSON] = ["status": .string(status)]
        if !reason.isEmpty { fields["reason"] = .string(reason) }
        return .object(fields)
    }
}
