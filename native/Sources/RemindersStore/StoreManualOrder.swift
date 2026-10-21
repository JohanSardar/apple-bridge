import Foundation

public enum StoreManualOrder: Equatable, Sendable {
    case supported([UUID: Int])
    case unsupported

    public func changedIdentifiers(from previous: Self) -> Set<UUID>? {
        switch (previous, self) {
        case (.unsupported, .unsupported): return []
        case (.supported(let before), .supported(let after)):
            var changed: Set<UUID> = []
            for (identifier, position) in before where after[identifier] != position { changed.insert(identifier) }
            for (identifier, position) in after where before[identifier] != position { changed.insert(identifier) }
            return changed
        default: return nil
        }
    }

    public static func decode(_ data: Data?) -> Self {
        guard let data, data.count <= 16_777_216,
              let values = try? JSONDecoder().decode([String].self, from: data) else { return .unsupported }
        var positions: [UUID: Int] = [:]
        positions.reserveCapacity(values.count)
        for (index, value) in values.enumerated() {
            guard value.count == 36, let uuid = UUID(uuidString: value),
                  uuid.uuidString.caseInsensitiveCompare(value) == .orderedSame,
                  positions.updateValue(index, forKey: uuid) == nil else { return .unsupported }
        }
        return .supported(positions)
    }
}
