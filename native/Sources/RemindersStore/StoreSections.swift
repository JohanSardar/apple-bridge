import Foundation

public struct StoreSections: Equatable, Sendable {
    public let order: [UUID]
    public let membership: [UUID: UUID]

    public static func decode(order: Data?, membership: Data?) -> Self? {
        guard let order, let membership,
              let names = try? JSONDecoder().decode([String].self, from: order),
              let members = try? JSONDecoder().decode([String: String].self, from: membership) else { return nil }
        let ids = names.compactMap(UUID.init(uuidString:))
        guard ids.count == names.count, Set(ids).count == ids.count else { return nil }
        var mapped: [UUID: UUID] = [:]
        for (item, section) in members {
            guard let item = UUID(uuidString: item), let section = UUID(uuidString: section), ids.contains(section), mapped[item] == nil else { return nil }
            mapped[item] = section
        }
        return Self(order: ids, membership: mapped)
    }
}
