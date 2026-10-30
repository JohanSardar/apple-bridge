import Foundation
import SourceSupport

public struct StoreSections: Equatable, Sendable {
    public let order: [UUID]
    public let membership: [UUID: UUID]

    static func object(_ data: Data?) -> Any? {
        guard let data, data.count <= 16_777_216 else { return nil }
        if let value = try? JSONSerialization.jsonObject(with: data) { return value }
        if let value = try? PropertyListSerialization.propertyList(from: data, format: nil),
           (value as? [String: Any])?["$archiver"] == nil { return value }
        return try? NSKeyedUnarchiver.unarchivedObject(ofClasses: [NSArray.self, NSDictionary.self, NSString.self, NSUUID.self, NSSet.self], from: data)
    }

    static func uuid(_ value: Any) -> UUID? {
        if let value = value as? UUID { return value }
        guard let text = value as? String, text.count == 36 else { return nil }
        return UUID(uuidString: text)
    }

    public static func decode(order: Data?, membership: Data?) -> Self? {
        guard let values = object(order) as? [Any] else { return nil }
        let identifiers = values.compactMap(uuid)
        guard identifiers.count == values.count, Set(identifiers).count == identifiers.count else { return nil }
        var members: [UUID: UUID] = [:]
        func insert(_ rawReminder: Any, _ rawSection: Any) -> Bool {
            guard let reminder = uuid(rawReminder), let section = uuid(rawSection), identifiers.contains(section), members[reminder] == nil else { return false }
            members[reminder] = section
            return true
        }
        if membership == nil { return Self(order: identifiers, membership: [:]) }
        if let dictionary = object(membership) as? [AnyHashable: Any] {
            for (key, value) in dictionary {
                if let reminders = value as? [Any] {
                    guard reminders.allSatisfy({ insert($0, key) }) else { return nil }
                } else if !insert(key, value) { return nil }
            }
        } else if let pairs = object(membership) as? [Any], pairs.count.isMultiple(of: 2) {
            for index in stride(from: 0, to: pairs.count, by: 2) {
                if let reminders = pairs[index + 1] as? [Any] {
                    guard reminders.allSatisfy({ insert($0, pairs[index]) }) else { return nil }
                } else if !insert(pairs[index], pairs[index + 1]) { return nil }
            }
        } else { return nil }
        return Self(order: identifiers, membership: members)
    }

    static func validated(_ list: StoreListRow, snapshot: TypedStoreSnapshot) -> Self? {
        let sections = snapshot.sections.values.filter { $0.list == list.identity.key }
        if sections.isEmpty, list.sectionOrder == nil, list.sectionMembership == nil { return Self(order: [], membership: [:]) }
        guard let decoded = decode(order: list.sectionOrder, membership: list.sectionMembership) else { return nil }
        let identities = sections.compactMap { RemindersStoreProjection.identifier($0.identity).flatMap(UUID.init(uuidString:)) }
        guard identities.count == sections.count, Set(identities) == Set(decoded.order) else { return nil }
        let reminders = Set(snapshot.reminders.values.filter { $0.list == list.identity.key }.compactMap { RemindersStoreProjection.reminderIdentifier($0).flatMap(UUID.init(uuidString:)) })
        guard Set(decoded.membership.keys).isSubset(of: reminders) else { return nil }
        return decoded
    }

    static func fields(_ list: StoreListRow, snapshot: TypedStoreSnapshot) -> JSON? {
        guard let decoded = validated(list, snapshot: snapshot) else { return nil }
        var byID: [UUID: StoreSectionRow] = [:]
        for section in snapshot.sections.values where section.list == list.identity.key {
            guard let id = RemindersStoreProjection.identifier(section.identity).flatMap(UUID.init(uuidString:)), byID[id] == nil else { return nil }
            byID[id] = section
        }
        return .array(decoded.order.enumerated().map { index, id in
            .object(["id": .string(id.uuidString), "title": .string(byID[id]?.name ?? ""), "kind": .string("store"), "order": .number(Double(index)), "reminder_ids": .array(decoded.membership.filter { $0.value == id }.keys.sorted { $0.uuidString < $1.uuidString }.map { RemindersStoreSnapshot.reference("reminders.reminder", $0.uuidString) })])
        })
    }
}
