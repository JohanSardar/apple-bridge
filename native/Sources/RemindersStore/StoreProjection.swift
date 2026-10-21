import Foundation
import SourceSupport

public enum RemindersStoreProjection {
    public static let fields: Set<String> = ["store_identifier", "flagged", "parent_id", "section_id", "tags", "manual_order", "created_at", "early_reminders", "assignment", "store_alarms", "store_recurrence", "attachment_ids", "badge_emblem", "is_group", "display_order", "pinned", "pinned_at", "sorting_style", "categorize_groceries", "grocery_locale", "sharing_status", "shared_owner_name", "smart_list_type", "smart_filter", "sections", "store_color", "store_status", "store_coverage"]

    public static func baseData(_ data: JSON) -> JSON { .object(data.object.filter { !fields.contains($0.key) }) }

    public static func baseObservation(_ observation: JSON) -> JSON {
        var result = observation.object
        if result["data"] != nil { result["data"] = baseData(observation["data"]) }
        return .object(result)
    }

    static func identifier(_ identity: StoreIdentity) -> String? { identity.uuid?.uuidString ?? identity.textIdentifier }

    static func reminderIdentifier(_ row: StoreReminderRow?) -> String? {
        guard let row, let key = row.calendarItemIdentifier, key == identifier(row.identity) else { return nil }
        return key
    }

    public static func unavailable(_ collection: String) -> JSON {
        var fields: [String: JSON] = ["store_status": .string("unavailable")]
        var coverage: [String: JSON] = [:]
        for key in collection == "reminders.reminder" ? reminderFields : listFields {
            fields[key] = .null
            coverage[key] = RemindersStoreSnapshot.coverage("unavailable", reason: "unique_exact_store_identity_unavailable")
        }
        fields["store_coverage"] = .object(coverage)
        return .object(fields)
    }

    private static let reminderFields = ["store_identifier", "flagged", "parent_id", "section_id", "tags", "manual_order", "created_at", "early_reminders", "assignment", "store_alarms", "store_recurrence", "attachment_ids"]
    private static let listFields = ["store_identifier", "parent_id", "badge_emblem", "is_group", "display_order", "pinned", "pinned_at", "sorting_style", "categorize_groceries", "grocery_locale", "sharing_status", "shared_owner_name", "smart_list_type", "smart_filter", "sections", "store_color"]

    static func reminder(_ row: StoreReminderRow, snapshot: TypedStoreSnapshot, columns: Set<String>, manualOrder: StoreManualOrder = .unsupported) -> JSON {
        let parent = row.parent.flatMap { snapshot.reminders[$0] }.flatMap(reminderIdentifier)
        var fields: [String: JSON] = ["store_identifier": value(identifier(row.identity)), "flagged": row.flagged.map(JSON.bool) ?? .null, "parent_id": parent.map { RemindersStoreSnapshot.reference("reminders.reminder", $0) } ?? .null, "created_at": row.created.map { .string(timestamp(Date(timeIntervalSinceReferenceDate: $0))) } ?? .null, "store_status": .string("partial")]
        var coverage: [String: JSON] = [:]
        for key in reminderFields where fields[key] == nil {
            fields[key] = .null
            coverage[key] = RemindersStoreSnapshot.coverage("unsupported", reason: "store_decoder_unverified")
        }
        for (field, column) in [("store_identifier", "ZIDENTIFIER"), ("flagged", "ZFLAGGED"), ("parent_id", "ZPARENTREMINDER"), ("created_at", "ZCREATIONDATE")] {
            coverage[field] = RemindersStoreSnapshot.coverage(columns.contains(column) ? "complete" : "unsupported")
        }
        if case .supported(let positions) = manualOrder,
           let identity = identifier(row.identity).flatMap(UUID.init(uuidString:)) {
            fields["manual_order"] = positions[identity].map { .number(Double($0)) } ?? .null
            coverage["manual_order"] = RemindersStoreSnapshot.coverage(positions[identity] == nil ? "partial" : "complete", reason: positions[identity] == nil ? "reminder_not_in_ordering" : "")
        }
        if row.flagged == nil, columns.contains("ZFLAGGED") { coverage["flagged"] = RemindersStoreSnapshot.coverage("unavailable") }
        if let key = row.parent, key != 0, parent == nil { coverage["parent_id"] = RemindersStoreSnapshot.coverage("partial", reason: "parent_identity_unavailable") }
        fields["store_coverage"] = .object(coverage)
        return .object(fields)
    }

    static func list(_ row: StoreListRow, snapshot: TypedStoreSnapshot, columns: Set<String>) -> JSON {
        var fields: [String: JSON] = ["store_identifier": value(identifier(row.identity)), "badge_emblem": value(row.emblem), "display_order": row.displayOrder.map(JSON.number) ?? .null, "sorting_style": scalar(row.sortingStyle), "grocery_locale": value(row.groceryLocale), "sharing_status": row.sharingStatus.map { .number(Double($0)) } ?? .null, "shared_owner_name": value(row.sharedOwner), "smart_list_type": scalar(row.smartType), "is_group": row.isGroup.map(JSON.bool) ?? .null, "pinned": row.pinned.map(JSON.bool) ?? .null, "categorize_groceries": row.groceries.map(JSON.bool) ?? .null, "pinned_at": row.pinnedDate.map { .string(timestamp(Date(timeIntervalSinceReferenceDate: $0))) } ?? .null, "store_status": .string("partial")]
        let candidates = Set([row.parent, row.alternateParent].compactMap { $0 }.filter { $0 != 0 })
        let parent = candidates.count == 1 ? candidates.first.flatMap { snapshot.lists[$0] }.flatMap { identifier($0.identity) } : nil
        fields["parent_id"] = parent.map { RemindersStoreSnapshot.reference("reminders.list", $0) } ?? .null
        var coverage: [String: JSON] = [:]
        for key in listFields where fields[key] == nil {
            fields[key] = .null
            coverage[key] = RemindersStoreSnapshot.coverage("unsupported", reason: "store_decoder_unverified")
        }
        for (field, column) in [("store_identifier", "ZIDENTIFIER"), ("badge_emblem", "ZBADGEEMBLEM"), ("display_order", "ZDADISPLAYORDER"), ("sorting_style", "ZSORTINGSTYLE"), ("grocery_locale", "ZGROCERYLOCALEID"), ("sharing_status", "ZSHARINGSTATUS"), ("shared_owner_name", "ZSHAREDOWNERNAME"), ("smart_list_type", "ZSMARTLISTTYPE"), ("is_group", "ZISGROUP"), ("pinned", "ZISPINNEDBYCURRENTUSER"), ("categorize_groceries", "ZSHOULDCATEGORIZEGROCERYITEMS"), ("pinned_at", "ZPINNEDDATE")] {
            coverage[field] = RemindersStoreSnapshot.coverage(columns.contains(column) ? "complete" : "unsupported")
        }
        coverage["parent_id"] = RemindersStoreSnapshot.coverage(candidates.count > 1 ? "unsupported" : !candidates.isEmpty && parent == nil ? "partial" : columns.contains("ZPARENTLIST") || columns.contains("ZPARENTLIST1") ? "complete" : "unsupported")
        fields["store_coverage"] = .object(coverage)
        return .object(fields)
    }

    private static func value(_ value: String?) -> JSON { value.map(JSON.string) ?? .null }

    private static func scalar(_ value: StoreScalar?) -> JSON {
        switch value {
        case .integer(let value): .number(Double(value))
        case .real(let value): .number(value)
        case .text(let value): .string(value)
        case nil: .null
        }
    }
}
